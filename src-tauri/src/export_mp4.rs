//! Cut MP4 export: feed the kept intervals to ffmpeg via the concat demuxer.
//! A temp list file describes each kept range as a `file` + `inpoint` +
//! `outpoint` triple, ffmpeg seeks to each in turn and re-encodes (libx264 +
//! aac). Lossless smart-cut is deferred.
//!
//! Why concat demuxer and not `filter_complex select`: building a single
//! `select=between(t,a,b)+between(t,c,d)+...` expression scales linearly with
//! the number of keeps. Past a few hundred intervals ffmpeg's expression
//! parser fails to allocate the parse tree ("Cannot allocate memory" inside
//! AVFilterGraph init), killing the export. The concat demuxer reads a flat
//! list and has no such limit.
//!
//! Progress is parsed from ffmpeg stderr `out_time_us=` lines (emitted via
//! `-progress pipe:2`). Total target duration is the sum of kept intervals
//! since the output stream's wall-clock is shorter than the source.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{anyhow, bail, Context, Result};

use crate::cutlist::CutList;
use crate::sync::lock;

pub struct ExportProgress {
    pub pct: f32,
    pub message: String,
}

/// Quality preset chosen by the user. `Source` targets the source's own
/// bitrate so the export lands at roughly the same size per second as the
/// input; the others are fixed libx264 CRF levels (lower = higher quality +
/// bigger file) with an audio bitrate that scales with them.
#[derive(Debug, Clone, Copy)]
pub enum Quality {
    Source,
    High,
    Medium,
    Small,
}

/// CRF used for `Quality::Source` when the source bitrate is unknown.
const SOURCE_FALLBACK_CRF: u8 = 20;

impl Quality {
    fn crf(self) -> u8 {
        match self {
            Quality::Source => SOURCE_FALLBACK_CRF,
            Quality::High => 18,
            Quality::Medium => 22,
            Quality::Small => 26,
        }
    }
    fn audio_bitrate(self) -> &'static str {
        match self {
            Quality::Source | Quality::High => "192k",
            Quality::Medium => "128k",
            Quality::Small => "96k",
        }
    }
}

/// What the export needs to know about the source to match it.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceSpec {
    /// Display size (rotation already applied). Zero when unknown.
    pub width: u32,
    pub height: u32,
    pub video_bitrate: Option<u64>,
    pub audio_bitrate: Option<u64>,
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u32>,
}

impl From<&crate::probe::MediaInfo> for SourceSpec {
    fn from(info: &crate::probe::MediaInfo) -> Self {
        let (width, height) = info.display_size();
        Self {
            width,
            height,
            video_bitrate: info.video_bitrate,
            audio_bitrate: info.audio_bitrate,
            audio_sample_rate: info.audio_sample_rate,
            audio_channels: info.audio_channels,
        }
    }
}

/// Output resolution. `Source` keeps the original dimensions; the numeric
/// variants cap the *short* side at that size (so 720p of a 1080x1920
/// portrait clip is 720x1280, not 404x720) and never upscale.
#[derive(Debug, Clone, Copy)]
pub enum Resolution {
    Source,
    P1080,
    P720,
    P480,
}

impl Resolution {
    fn target_height(self) -> Option<u32> {
        match self {
            Resolution::Source => None,
            Resolution::P1080 => Some(1080),
            Resolution::P720 => Some(720),
            Resolution::P480 => Some(480),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub quality: Quality,
    pub resolution: Resolution,
    pub has_audio: bool,
    /// Rendered ASS subtitle text to burn into the output via ffmpeg's `ass`
    /// filter. `None` skips subtitle filtering entirely.
    pub subtitle_ass: Option<String>,
    pub source: SourceSpec,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            quality: Quality::Source,
            resolution: Resolution::Source,
            has_audio: true,
            subtitle_ass: None,
            source: SourceSpec::default(),
        }
    }
}

pub fn export(
    ffmpeg: &Path,
    source: &Path,
    output: &Path,
    cutlist: &CutList,
    options: ExportOptions,
    cancel: Arc<AtomicBool>,
    on_progress: impl FnMut(ExportProgress) + Send + 'static,
) -> Result<()> {
    cutlist.ensure_exportable()?;
    let kept_total: f64 = cutlist.total_kept_duration();

    let list_path = concat_list_path();
    std::fs::write(&list_path, build_concat_list(source, cutlist))
        .with_context(|| format!("writing concat list at {}", list_path.display()))?;
    // Defer file removal so it persists even if we early-return on error —
    // makes failures debuggable. Cleaned up at the bottom of the happy path.

    let subtitle_path = match &options.subtitle_ass {
        Some(ass) => {
            let path = subtitle_temp_path();
            std::fs::write(&path, ass)
                .with_context(|| format!("writing subtitle temp file at {}", path.display()))?;
            Some(path)
        }
        None => None,
    };

    // Relative paths would resolve against the ffmpeg working directory,
    // which is moved to the subtitle's folder below; pin the output first.
    let output = if output.is_relative() {
        std::env::current_dir()
            .context("resolving output path")?
            .join(output)
    } else {
        output.to_path_buf()
    };

    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-nostats",
        "-progress",
        "pipe:2",
        "-loglevel",
        "error",
    ])
    .args(["-f", "concat", "-safe", "0"])
    .arg("-i")
    .arg(&list_path)
    .args(["-c:v", "libx264", "-preset", "veryfast"]);
    let out_size = output_size(
        options.source.width,
        options.source.height,
        options.resolution.target_height(),
    );
    cmd.args(video_rate_args(
        options.quality,
        scaled_bitrate(&options.source, out_size),
    ));
    // Scale and/or burn in subtitles via -vf. Concat demuxer produces
    // continuous timestamps across segments on its own, no setpts needed. Both filters, when
    // present, must be comma-joined into one -vf chain — ffmpeg takes a
    // single filter graph, not one flag per filter.
    let mut filters: Vec<String> = Vec::new();
    if let Some(path) = &subtitle_path {
        let (dir, filter) = subtitle_filter(path)?;
        cmd.current_dir(dir);
        filters.push(filter);
    }
    if let Some((w, h)) = out_size {
        filters.push(format!("scale={w}:{h}"));
    } else if let Some(h) = options.resolution.target_height() {
        // Source size unknown: fall back to letting ffmpeg work it out,
        // capped at the target height and never upscaling.
        filters.push(format!("scale=-2:'min({h},ih)'"));
    }
    if !filters.is_empty() {
        cmd.args(["-vf", &filters.join(",")]);
    }
    if options.has_audio {
        cmd.arg("-c:a")
            .arg("aac")
            .args(audio_args(options.quality, &options.source));
    } else {
        cmd.arg("-an");
    }
    cmd.arg(&output);
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());

    let mut child = cmd.spawn().context("spawning ffmpeg for export")?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stderr unavailable"))?;

    // Keep a rolling buffer of the last few non-progress stderr lines so we
    // can surface ffmpeg's actual error message when the encode fails.
    // Anything matching `key=value` is a progress field; the real diagnostics
    // are the plain-prose lines.
    let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_tail_writer = stderr_tail.clone();
    let cancel_thread = cancel.clone();
    let reader = thread::spawn(move || -> Result<()> {
        // The callback is owned outright by this thread, so it needs no
        // synchronisation — it used to sit behind a Mutex that nothing else
        // could ever contend for.
        let mut on_progress = on_progress;
        let buf = BufReader::new(stderr);
        for line in buf.lines() {
            let line = line.context("reading ffmpeg stderr")?;
            if cancel_thread.load(Ordering::SeqCst) {
                break;
            }
            if let Some(us) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = us.trim().parse::<i64>() {
                    let seconds = us.max(0) as f64 / 1_000_000.0;
                    let pct = ((seconds / kept_total) * 100.0).clamp(0.0, 99.0);
                    let msg = format!("encoded {:.2}s of {:.2}s kept", seconds, kept_total);
                    on_progress(ExportProgress {
                        pct: pct as f32,
                        message: msg,
                    });
                }
                continue;
            }
            // Only keep the last 20 diagnostic lines — enough to identify the
            // failure without dumping every progress key.
            if !is_progress_line(&line) && !line.trim().is_empty() {
                let mut tail = lock(&stderr_tail_writer);
                tail.push(line);
                if tail.len() > 20 {
                    let drop = tail.len() - 20;
                    tail.drain(0..drop);
                }
            }
        }
        Ok(())
    });

    // Cancellation: poll the flag and kill the child if set.
    let outcome: Result<()> = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            break Err(anyhow!("export cancelled"));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = reader.join();
                if !status.success() {
                    let tail = lock(&stderr_tail);
                    let detail = if tail.is_empty() {
                        String::new()
                    } else {
                        format!("\nffmpeg said:\n{}", tail.join("\n"))
                    };
                    break Err(anyhow!("ffmpeg exited with {status}{detail}"));
                }
                break Ok(());
            }
            Ok(None) => thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => break Err(anyhow!("waiting on ffmpeg: {e}")),
        }
    };

    // Only clean up the concat list (and subtitle temp file) on success; leave
    // them on disk on failure so the user (or a bug report) can inspect what
    // we asked ffmpeg to do.
    if outcome.is_ok() {
        let _ = std::fs::remove_file(&list_path);
        if let Some(path) = &subtitle_path {
            let _ = std::fs::remove_file(path);
        }
    } else {
        // Killing ffmpeg mid-encode leaves a truncated, unplayable mp4 at the
        // path the user chose — and if they were overwriting an earlier
        // export, that file is now gone and replaced with the stub. Remove it
        // so a cancelled or failed export leaves nothing behind pretending to
        // be a result.
        let _ = std::fs::remove_file(output);
    }
    outcome
}

/// libx264 rate-control flags. `Source` with a known bitrate encodes to that
/// average bitrate (VBV-capped at 1.5x so a busy scene can't balloon), which
/// keeps the export's size proportional to the source's. Everything else is
/// constant-quality CRF.
fn video_rate_args(quality: Quality, video_bitrate: Option<u64>) -> Vec<String> {
    match (quality, video_bitrate) {
        (Quality::Source, Some(b)) => vec![
            "-b:v".into(),
            b.to_string(),
            "-maxrate".into(),
            (b * 3 / 2).to_string(),
            "-bufsize".into(),
            (b * 2).to_string(),
        ],
        _ => vec!["-crf".into(), quality.crf().to_string()],
    }
}

/// Source video bitrate adjusted for a downscale: same bits per pixel, so a
/// 720p export of a 1080p source is proportionally smaller instead of
/// spending the full source bitrate on fewer pixels.
fn scaled_bitrate(source: &SourceSpec, out_size: Option<(u32, u32)>) -> Option<u64> {
    let b = source.video_bitrate?;
    let src_px = source.width as u64 * source.height as u64;
    match out_size {
        Some((w, h)) if src_px > 0 => {
            Some((b as u128 * (w as u128 * h as u128) / src_px as u128) as u64)
        }
        _ => Some(b),
    }
}

/// AAC flags. `Source` mirrors the source's bitrate (clamped to a sane AAC
/// range), sample rate, and channel count; the presets use a fixed bitrate.
fn audio_args(quality: Quality, source: &SourceSpec) -> Vec<String> {
    let mut args = Vec::new();
    match quality {
        Quality::Source => {
            let bitrate = source
                .audio_bitrate
                .map(|b| b.clamp(64_000, 320_000).to_string())
                .unwrap_or_else(|| quality.audio_bitrate().to_string());
            args.extend(["-b:a".to_string(), bitrate]);
            if let Some(rate) = source.audio_sample_rate {
                args.extend(["-ar".to_string(), rate.to_string()]);
            }
            if let Some(ch) = source.audio_channels {
                args.extend(["-ac".to_string(), ch.to_string()]);
            }
        }
        _ => args.extend(["-b:a".to_string(), quality.audio_bitrate().to_string()]),
    }
    args
}

/// Output frame size for a source of `w`x`h` (display orientation), or `None`
/// when the source size can be passed through untouched.
///
/// `target` caps the short side, aspect ratio preserved, never upscaling.
/// Both sides are rounded to even numbers — libx264's yuv420p refuses odd
/// dimensions, so an odd-sized source gets trimmed by one pixel even when no
/// target is set. Unknown size (0) returns `None`.
fn output_size(w: u32, h: u32, target: Option<u32>) -> Option<(u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let even = |v: f64| ((v / 2.0).round() as u32 * 2).max(2);
    let short = w.min(h);
    let (out_w, out_h) = match target {
        Some(t) if t < short => {
            let scale = t as f64 / short as f64;
            if w <= h {
                (even(t as f64), even(h as f64 * scale))
            } else {
                (even(w as f64 * scale), even(t as f64))
            }
        }
        _ => (w - w % 2, h - h % 2),
    };
    if (out_w, out_h) == (w, h) {
        None
    } else {
        Some((out_w.max(2), out_h.max(2)))
    }
}

/// Is this stderr line one of ffmpeg's `-progress` fields rather than a
/// diagnostic worth showing the user?
///
/// The progress stream is strictly `snake_case_key=value`. Testing for a bare
/// '=' anywhere in the line also swallowed real errors — ffmpeg says things
/// like "maybe incorrect parameters such as bit_rate, rate, width or height"
/// and quotes filter expressions containing '=' — which left failed exports
/// reporting nothing but the exit status.
fn is_progress_line(line: &str) -> bool {
    let Some((key, _)) = line.split_once('=') else {
        return false;
    };
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Build a concat demuxer list (UTF-8, LF line endings) repeating the source
/// path for every kept interval with `inpoint`/`outpoint` directives.
///
/// The path is wrapped in single quotes; embedded single quotes are escaped as
/// `'\''` (close-quote, escaped-quote, reopen-quote — the standard POSIX
/// shell idiom that ffmpeg's concat parser also accepts). Backslashes in
/// Windows paths are literal inside single quotes; no escaping needed.
fn build_concat_list(source: &Path, cutlist: &CutList) -> String {
    let escaped = source.to_string_lossy().replace('\'', "'\\''");
    let mut out = String::new();
    out.push_str("ffconcat version 1.0\n");
    for c in cutlist.kept_intervals() {
        out.push_str(&format!("file '{escaped}'\n"));
        out.push_str(&format!("inpoint {:.6}\n", c.start));
        out.push_str(&format!("outpoint {:.6}\n", c.end));
    }
    out
}

/// Unique path inside the OS temp dir. PID + a process-local atomic counter
/// guarantees no collision between concurrent exports (the cli never runs them
/// in parallel today, but cheap insurance).
fn concat_list_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("autocut-concat-{}-{}.txt", std::process::id(), id))
}

/// Same uniqueness scheme as [`concat_list_path`], its own counter so the two
/// never collide with each other either.
fn subtitle_temp_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("autocut-subs-{}-{}.ass", std::process::id(), id))
}

/// Build the `ass=` filter for a subtitle file, plus the directory ffmpeg
/// must run in for it to resolve.
///
/// The filter gets the bare file name, never the full path. A Windows path
/// like `C:\Users\...\Temp\x.ass` goes through two rounds of ffmpeg
/// unescaping (filtergraph, then the filter's own option parser), and the
/// drive-letter `:` ends up read as an option separator, so `ass` tries to
/// open a file called `C`. Running ffmpeg from the subtitle's folder and
/// naming only the file sidesteps escaping entirely. The temp name is
/// `autocut-subs-<pid>-<n>.ass`, which has no characters ffmpeg treats
/// specially.
fn subtitle_filter(path: &Path) -> Result<(PathBuf, String)> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("subtitle path has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("subtitle path has no file name: {}", path.display()))?;
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("subtitle file name needs no escaping but got {name:?}");
    }
    Ok((dir.to_path_buf(), format!("ass={name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutlist::{Cut, CutKind};

    #[test]
    fn concat_list_emits_one_entry_per_kept_interval() {
        let cl = CutList {
            source_duration: 10.0,
            intervals: vec![
                Cut {
                    start: 0.0,
                    end: 1.0,
                    kind: CutKind::Keep,
                },
                Cut {
                    start: 1.0,
                    end: 2.0,
                    kind: CutKind::Remove,
                },
                Cut {
                    start: 2.0,
                    end: 3.5,
                    kind: CutKind::Keep,
                },
            ],
        };
        let list = build_concat_list(Path::new("/tmp/clip.mp4"), &cl);
        assert!(list.starts_with("ffconcat version 1.0\n"), "{list}");
        assert_eq!(list.matches("file '/tmp/clip.mp4'").count(), 2);
        assert!(list.contains("inpoint 0.000000"));
        assert!(list.contains("outpoint 1.000000"));
        assert!(list.contains("inpoint 2.000000"));
        assert!(list.contains("outpoint 3.500000"));
        // The removed range must not appear as a keep entry.
        assert!(!list.contains("inpoint 1.000000\noutpoint 2.000000"));
    }

    #[test]
    fn concat_list_is_just_a_header_when_no_keeps() {
        let cl = CutList {
            source_duration: 5.0,
            intervals: vec![],
        };
        let list = build_concat_list(Path::new("/tmp/clip.mp4"), &cl);
        assert_eq!(list, "ffconcat version 1.0\n");
    }

    #[test]
    fn concat_list_escapes_single_quotes_in_paths() {
        let cl = CutList {
            source_duration: 5.0,
            intervals: vec![Cut {
                start: 0.0,
                end: 1.0,
                kind: CutKind::Keep,
            }],
        };
        let list = build_concat_list(Path::new("/tmp/it's mine.mp4"), &cl);
        assert!(list.contains(r"file '/tmp/it'\''s mine.mp4'"), "{list}");
    }

    #[test]
    fn concat_list_preserves_windows_backslash_paths() {
        let cl = CutList {
            source_duration: 5.0,
            intervals: vec![Cut {
                start: 0.0,
                end: 1.0,
                kind: CutKind::Keep,
            }],
        };
        let list = build_concat_list(Path::new(r"C:\Users\me\my video.mp4"), &cl);
        // Backslashes are literal inside single quotes; nothing should escape them.
        assert!(list.contains(r"file 'C:\Users\me\my video.mp4'"), "{list}");
    }

    #[test]
    fn subtitle_filter_uses_bare_name_for_windows_temp_path() {
        let path = PathBuf::from(r"C:\Users\me\AppData\Local\Temp").join("autocut-subs-1-0.ass");
        let (dir, filter) = subtitle_filter(&path).unwrap();
        assert_eq!(filter, "ass=autocut-subs-1-0.ass");
        assert_eq!(dir, PathBuf::from(r"C:\Users\me\AppData\Local\Temp"));
        assert!(!filter.contains('\\'));
    }

    #[test]
    fn subtitle_temp_name_needs_no_escaping() {
        let (_, filter) = subtitle_filter(&subtitle_temp_path()).unwrap();
        assert!(filter.starts_with("ass=autocut-subs-"), "{filter}");
    }

    #[test]
    fn output_size_keeps_an_even_source_untouched() {
        assert_eq!(output_size(1920, 1080, None), None);
        assert_eq!(output_size(1080, 1920, None), None);
    }

    #[test]
    fn output_size_caps_the_short_side_of_portrait_video() {
        assert_eq!(output_size(1080, 1920, Some(720)), Some((720, 1280)));
        assert_eq!(output_size(1920, 1080, Some(720)), Some((1280, 720)));
    }

    #[test]
    fn output_size_never_upscales() {
        assert_eq!(output_size(1280, 720, Some(1080)), None);
        assert_eq!(output_size(1080, 1920, Some(1080)), None);
    }

    #[test]
    fn output_size_trims_odd_dimensions_to_even() {
        assert_eq!(output_size(1921, 1080, None), Some((1920, 1080)));
        assert_eq!(output_size(1081, 1921, Some(720)), Some((720, 1280)));
    }

    #[test]
    fn output_size_is_none_when_source_size_unknown() {
        assert_eq!(output_size(0, 0, Some(720)), None);
    }

    #[test]
    fn source_quality_targets_the_source_bitrate() {
        assert_eq!(
            video_rate_args(Quality::Source, Some(8_000_000)),
            ["-b:v", "8000000", "-maxrate", "12000000", "-bufsize", "16000000"]
        );
    }

    #[test]
    fn source_quality_falls_back_to_crf_without_a_bitrate() {
        assert_eq!(video_rate_args(Quality::Source, None), ["-crf", "20"]);
        assert_eq!(video_rate_args(Quality::Medium, Some(1)), ["-crf", "22"]);
    }

    #[test]
    fn downscaling_scales_the_bitrate_by_pixel_count() {
        let src = SourceSpec {
            width: 1080,
            height: 1920,
            video_bitrate: Some(9_000_000),
            ..Default::default()
        };
        assert_eq!(scaled_bitrate(&src, None), Some(9_000_000));
        assert_eq!(scaled_bitrate(&src, Some((720, 1280))), Some(4_000_000));
        assert_eq!(scaled_bitrate(&SourceSpec::default(), None), None);
    }

    #[test]
    fn source_quality_mirrors_the_source_audio() {
        let src = SourceSpec {
            audio_bitrate: Some(256_000),
            audio_sample_rate: Some(44_100),
            audio_channels: Some(1),
            ..Default::default()
        };
        assert_eq!(
            audio_args(Quality::Source, &src),
            ["-b:a", "256000", "-ar", "44100", "-ac", "1"]
        );
        assert_eq!(audio_args(Quality::Small, &src), ["-b:a", "96k"]);
    }

    #[test]
    fn concat_list_path_is_unique_per_call() {
        let a = concat_list_path();
        let b = concat_list_path();
        assert_ne!(a, b);
    }

    #[test]
    fn progress_fields_are_recognised() {
        for line in [
            "out_time_us=1234567",
            "frame=42",
            "bitrate=N/A",
            "speed=1.02x",
            "progress=continue",
        ] {
            assert!(is_progress_line(line), "{line} should be a progress field");
        }
    }

    #[test]
    fn diagnostics_containing_an_equals_sign_are_not_mistaken_for_progress() {
        // The tail buffer exists to surface ffmpeg's actual complaint when an
        // encode fails. Treating every line with an '=' in it as a progress
        // field threw away exactly the lines worth keeping, leaving the user
        // with a bare "ffmpeg exited with exit status: 1".
        for line in [
            "[libx264 @ 0x7f8] width not divisible by 2 (1921x1080)",
            "Error while opening encoder for output stream #0:0 - maybe incorrect parameters such as bit_rate, rate, width or height",
            "[Parsed_scale_0 @ 0x600] Error when evaluating the expression 'a=b'",
            "Conversion failed!",
            "",
        ] {
            assert!(
                !is_progress_line(line),
                "{line} should be kept as a diagnostic"
            );
        }
    }
}
