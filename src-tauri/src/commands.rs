//! Tauri command surface. Frontend talks to these via `invoke()`.
//!
//! Long-running operations (`detect_silence`, `export_mp4`) run on a worker
//! thread and emit progress events via the Tauri Emitter API. Frontend
//! subscribes via `listen('export-progress', ...)` etc.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::extract_pcm_with_cancel;
use crate::binaries::{ffmpeg_path, ffprobe_path, model_path};
use crate::cutlist::CutList;
use crate::export_fcpxml::{render as render_fcpxml, FcpxmlParams, Track};
use crate::export_mp4;
use crate::llm;
use crate::logstore;
use crate::oauth;
use crate::probe::{probe, MediaInfo};
use crate::queue::{Job, JobQueue};
use crate::settings::{self, AppSettings};
use crate::subtitle::{render_ass, render_srt, SubtitleStyle, VideoSize};
use crate::sync::lock;
use crate::timecode::parse_smpte;
use crate::timeline::map_to_output;
use crate::transcribe::transcribe;
use crate::vad::{score_chunks, segments_from_scores, VadParams};
use crate::waveform::waveform_from_samples;

/// Render an anyhow::Error including the full chain of `with_context`
/// messages. Plain `.to_string()` would only show the outermost message,
/// hiding the actual root cause (e.g. "spawning ffprobe at ...: No such
/// file or directory" instead of just "spawning ffprobe at ...").
fn fmt_err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

pub struct AppState {
    /// Single-job cancellation flag. Set by `cancel_export`, polled by the
    /// export worker. The single-video editor (hidden from the primary UI,
    /// kept for later) only ever runs one job of each kind at a time.
    pub export_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub detect_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub waveform_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub subtitle_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub llm_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pub analysis_cache: Arc<AnalysisCache>,
    /// The batch pipeline's job list. Unlike the single-slot cancel flags
    /// above, this supports any number of jobs — `queue::start` is what
    /// decides how many run at once.
    pub queue: Arc<JobQueue>,
    pub settings: Mutex<AppSettings>,
    pub app_data_dir: PathBuf,
}

impl AppState {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let settings = settings::load(&app_data_dir);
        Self {
            export_cancel: Mutex::new(None),
            detect_cancel: Mutex::new(None),
            waveform_cancel: Mutex::new(None),
            subtitle_cancel: Mutex::new(None),
            llm_cancel: Mutex::new(None),
            analysis_cache: Arc::new(AnalysisCache::default()),
            queue: Arc::new(JobQueue::default()),
            settings: Mutex::new(settings),
            app_data_dir,
        }
    }
}

/// Everything derived from one source file's audio that survives across
/// commands: the decoded PCM, and silero's per-chunk speech probabilities.
///
/// Both are keyed on the file's identity so switching sources invalidates
/// them, and both are `Arc` so a reader can drop the lock before doing any
/// real work with them.
#[derive(Default)]
pub struct AnalysisCache {
    inner: Mutex<Option<CachedAnalysis>>,
}

struct CachedAnalysis {
    key: CacheKey,
    samples: Arc<Vec<i16>>,
    /// Populated on the first detection for this source. `None` until then —
    /// loading a video computes the waveform, which needs the PCM but not the
    /// scores, and scoring an hour of audio is seconds of work nobody has
    /// asked for yet.
    scores: Option<Arc<Vec<f32>>>,
}

#[derive(Clone, Eq, PartialEq)]
struct CacheKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

impl CacheKey {
    fn from_path(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(fmt_err)?;
        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

const CANCELLED: &str = "audio extraction cancelled";

impl AnalysisCache {
    /// Decoded 16kHz mono PCM for the whole file, or a fresh uncached decode
    /// when a sub-range was asked for.
    fn samples(
        &self,
        ffmpeg: &Path,
        video: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Vec<i16>>, String> {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }

        let key = CacheKey::from_path(video)?;
        let mut guard = lock(&self.inner);
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        if let Some(entry) = guard.as_ref() {
            if entry.key == key {
                return Ok(entry.samples.clone());
            }
        }

        // Decoding with the lock held is deliberate. Loading a video kicks off
        // the waveform immediately and the user can hit detect a moment later;
        // both want the same PCM. Serialising here makes the second caller
        // wait for and then reuse the first one's decode rather than running a
        // redundant ffmpeg over the same file. The wait is interruptible: the
        // holder polls its own cancel flag inside ffmpeg's read loop.
        let samples =
            Arc::new(extract_pcm_with_cancel(ffmpeg, video, Some(cancel)).map_err(fmt_err)?);
        *guard = Some(CachedAnalysis {
            key,
            samples: samples.clone(),
            scores: None,
        });
        Ok(samples)
    }

    /// Silero's speech probability per 512-sample chunk.
    ///
    /// This is what makes moving a detection slider cheap: the probabilities
    /// depend only on the audio, so a re-detect after a parameter change is a
    /// cache hit plus a pass of pure arithmetic instead of ~112k model
    /// invocations for an hour of footage.
    fn scores(
        &self,
        ffmpeg: &Path,
        video: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<Arc<Vec<f32>>, String> {
        let samples = self.samples(ffmpeg, video, cancel.clone())?;

        let key = CacheKey::from_path(video)?;
        if let Some(entry) = lock(&self.inner).as_ref() {
            if entry.key == key {
                if let Some(scores) = &entry.scores {
                    return Ok(scores.clone());
                }
            }
        }

        // Scored without the lock. Unlike the decode above there is nothing to
        // share — only one detection job runs at a time — so holding it would
        // just block the waveform for the length of a model pass.
        let scores = Arc::new(score_chunks(&samples, Some(&cancel)).map_err(fmt_err)?);
        if let Some(entry) = lock(&self.inner).as_mut() {
            if entry.key == key {
                entry.scores = Some(scores.clone());
            }
        }
        Ok(scores)
    }
}

fn install_cancel_slot(slot: &Mutex<Option<Arc<AtomicBool>>>) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut guard = lock(slot);
    if let Some(previous) = guard.replace(cancel.clone()) {
        previous.store(true, Ordering::SeqCst);
    }
    cancel
}

fn clear_cancel_slot(slot: &Mutex<Option<Arc<AtomicBool>>>, cancel: &Arc<AtomicBool>) {
    let mut guard = lock(slot);
    if guard
        .as_ref()
        .map(|current| Arc::ptr_eq(current, cancel))
        .unwrap_or(false)
    {
        *guard = None;
    }
}

fn cancel_slot(slot: &Mutex<Option<Arc<AtomicBool>>>) {
    if let Some(flag) = lock(slot).take() {
        flag.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Deserialize)]
pub struct DetectParams {
    pub threshold: f32,
    pub min_silence_ms: u32,
    pub min_speech_ms: u32,
    pub pad: f64,
    /// Project-time second the analysed audio begins at. Non-zero when
    /// detection runs on a linked track that rolled after the reference; the
    /// resulting cutlist is always expressed in project time.
    #[serde(default)]
    pub source_offset: f64,
}

impl From<&DetectParams> for VadParams {
    fn from(p: &DetectParams) -> Self {
        VadParams {
            threshold: p.threshold,
            min_silence_ms: p.min_silence_ms,
            min_speech_ms: p.min_speech_ms,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DetectResult {
    pub cutlist: CutList,
}

fn resource_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok()
}

#[tauri::command]
pub async fn compute_waveform(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    target_bins: usize,
) -> Result<Vec<f32>, String> {
    let ffmpeg = ffmpeg_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let video = PathBuf::from(&path);
    let cancel = install_cancel_slot(&state.waveform_cancel);
    let cancel_for_worker = cancel.clone();
    let cache = state.analysis_cache.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let samples = cache.samples(&ffmpeg, &video, cancel_for_worker.clone())?;
        waveform_from_samples(
            samples.as_slice(),
            target_bins,
            Some(cancel_for_worker.as_ref()),
        )
        .map_err(fmt_err)
    })
    .await;
    clear_cancel_slot(&state.waveform_cancel, &cancel);
    let result = joined.map_err(fmt_err)?;
    result.map_err(fmt_err)
}

#[derive(Debug, Serialize)]
pub struct DiagnosticInfo {
    pub app_version: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
    pub ffmpeg_path: Option<String>,
    pub ffmpeg_exists: bool,
    pub ffprobe_path: Option<String>,
    pub ffprobe_exists: bool,
}

/// Snapshot of build/runtime info we ask the user to paste into bug reports
/// when something fails. No PII; just versions and paths.
#[tauri::command]
pub fn diagnostic_info(app: AppHandle) -> DiagnosticInfo {
    let rd = resource_dir(&app);
    let ffmpeg = ffmpeg_path(rd.as_deref()).ok();
    let ffprobe = ffprobe_path(rd.as_deref()).ok();
    DiagnosticInfo {
        app_version: env!("CARGO_PKG_VERSION"),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        ffmpeg_exists: ffmpeg.as_ref().map(|p| p.exists()).unwrap_or(false),
        ffmpeg_path: ffmpeg.map(|p| p.display().to_string()),
        ffprobe_exists: ffprobe.as_ref().map(|p| p.exists()).unwrap_or(false),
        ffprobe_path: ffprobe.map(|p| p.display().to_string()),
    }
}

#[tauri::command]
pub async fn open_video(app: AppHandle, path: String) -> Result<MediaInfo, String> {
    let info = open_media(app, path).await?;
    // The reference clip is what the player shows and what the mp4 export
    // encodes, so unlike a linked track it does have to carry picture.
    if !info.has_video {
        return Err(format!("no video stream in {}", info.path));
    }
    Ok(info)
}

#[derive(Debug, Serialize)]
pub struct TrackProbe {
    pub info: MediaInfo,
    pub offset: f64,
    pub offset_from_timecode: bool,
}

/// Probe a file that will ride along as a linked track. Audio-only is fine
/// here — a separate sound recorder's WAV is the whole point.
///
/// Also works out where the track belongs on the timeline. When both it and
/// the reference carry embedded timecode, the same wall-clock instant is the
/// one where their timecodes agree, so the track's media begins at project
/// time `trackTC - referenceTC`. Without timecode on both sides there is
/// nothing to derive it from, so it starts at zero for the user to correct.
#[tauri::command]
pub async fn open_track(
    app: AppHandle,
    path: String,
    reference_timecode: Option<String>,
    fps: f64,
) -> Result<TrackProbe, String> {
    let info = open_media(app, path).await?;
    let (offset, offset_from_timecode) = match (
        reference_timecode.as_deref(),
        info.start_timecode.as_deref(),
    ) {
        (Some(reference), Some(track)) => (
            parse_smpte(Some(track), fps) - parse_smpte(Some(reference), fps),
            true,
        ),
        _ => (0.0, false),
    };
    Ok(TrackProbe {
        info,
        offset,
        offset_from_timecode,
    })
}

async fn open_media(app: AppHandle, path: String) -> Result<MediaInfo, String> {
    let ffprobe = ffprobe_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let media = PathBuf::from(&path);
    tauri::async_runtime::spawn_blocking(move || probe(&ffprobe, &media))
        .await
        .map_err(fmt_err)?
        .map_err(fmt_err)
}

#[tauri::command]
pub async fn detect_silence(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    duration: f64,
    params: DetectParams,
) -> Result<DetectResult, String> {
    let ffmpeg = ffmpeg_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let video = PathBuf::from(&path);
    let pad = params.pad;
    let source_offset = params.source_offset;
    let vad_params: VadParams = (&params).into();
    let cancel = install_cancel_slot(&state.detect_cancel);
    let cancel_for_worker = cancel.clone();
    let cache = state.analysis_cache.clone();

    let joined = tauri::async_runtime::spawn_blocking(move || -> Result<DetectResult, String> {
        // Only the scoring step is expensive, and it's cached per source, so
        // every detect after the first is this cheap tail: re-segment the
        // probabilities under the new parameters and re-invert into a cutlist.
        let scores = cache.scores(&ffmpeg, &video, cancel_for_worker.clone())?;
        let segments = segments_from_scores(&scores, vad_params, source_offset);
        let cutlist = CutList::from_speech_segments(&segments, duration, pad);
        Ok(DetectResult { cutlist })
    })
    .await;
    clear_cancel_slot(&state.detect_cancel, &cancel);
    joined.map_err(fmt_err)?
}

#[derive(Debug, Deserialize)]
pub struct VideoSizeArgs {
    pub width: u32,
    pub height: u32,
}

impl From<VideoSizeArgs> for VideoSize {
    fn from(v: VideoSizeArgs) -> Self {
        VideoSize {
            width: v.width,
            height: v.height,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleFormat {
    Ass,
    Srt,
}

/// Transcribe `path`'s audio, re-base the captions onto the cut timeline, and
/// render them. Reuses the same cached PCM `detect_silence`/`compute_waveform`
/// already decoded — no second ffmpeg pass over the source.
#[tauri::command]
pub async fn generate_subtitles(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    cutlist: CutList,
    video: VideoSizeArgs,
    format: SubtitleFormat,
) -> Result<String, String> {
    let ffmpeg = ffmpeg_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let model = model_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let video: VideoSize = video.into();
    let source = PathBuf::from(&path);
    let cancel = install_cancel_slot(&state.subtitle_cancel);
    let cancel_for_worker = cancel.clone();
    let cache = state.analysis_cache.clone();

    let joined = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let samples = cache.samples(&ffmpeg, &source, cancel_for_worker.clone())?;
        let segments = transcribe(&model, &samples, cancel_for_worker.clone()).map_err(fmt_err)?;
        let output_segments = map_to_output(&cutlist, &segments);
        let style = SubtitleStyle::default();
        Ok(match format {
            SubtitleFormat::Ass => render_ass(&output_segments, video, &style),
            SubtitleFormat::Srt => render_srt(&output_segments),
        })
    })
    .await;
    clear_cancel_slot(&state.subtitle_cancel, &cancel);
    joined.map_err(fmt_err)?
}

#[tauri::command]
pub fn cancel_subtitles(state: State<'_, AppState>) -> Result<(), String> {
    cancel_slot(&state.subtitle_cancel);
    Ok(())
}

/// Write rendered subtitle text (ASS or SRT) to disk. The frontend has no
/// filesystem plugin of its own, so this is the same one-line `std::fs::write`
/// `export_fcpxml` already does for its XML string, just exposed for the
/// subtitle sidecar-file case (FCPXML export, or saving alongside an mp4).
#[tauri::command]
pub fn save_subtitles(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(fmt_err)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportQuality {
    High,
    Medium,
    Small,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportResolution {
    Source,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "480p")]
    P480,
}

#[derive(Debug, Deserialize)]
pub struct ExportMp4Args {
    pub source: String,
    pub output: String,
    pub cutlist: CutList,
    #[serde(default = "default_quality")]
    pub quality: ExportQuality,
    #[serde(default = "default_resolution")]
    pub resolution: ExportResolution,
    #[serde(default = "default_has_audio")]
    pub has_audio: bool,
    /// Rendered ASS subtitle text (from `generate_subtitles`) to burn into
    /// the output. `None`/absent skips subtitle filtering.
    #[serde(default)]
    pub subtitle_ass: Option<String>,
}

fn default_quality() -> ExportQuality {
    ExportQuality::Medium
}
fn default_resolution() -> ExportResolution {
    ExportResolution::Source
}
fn default_has_audio() -> bool {
    true
}

impl From<ExportQuality> for export_mp4::Quality {
    fn from(q: ExportQuality) -> Self {
        match q {
            ExportQuality::High => export_mp4::Quality::High,
            ExportQuality::Medium => export_mp4::Quality::Medium,
            ExportQuality::Small => export_mp4::Quality::Small,
        }
    }
}

impl From<ExportResolution> for export_mp4::Resolution {
    fn from(r: ExportResolution) -> Self {
        match r {
            ExportResolution::Source => export_mp4::Resolution::Source,
            ExportResolution::P1080 => export_mp4::Resolution::P1080,
            ExportResolution::P720 => export_mp4::Resolution::P720,
            ExportResolution::P480 => export_mp4::Resolution::P480,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ExportProgressEvent {
    pct: f32,
    message: String,
}

#[tauri::command]
pub async fn export_mp4(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ExportMp4Args,
) -> Result<(), String> {
    let ffmpeg = ffmpeg_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let source = PathBuf::from(&args.source);
    let output = PathBuf::from(&args.output);

    let cancel = install_cancel_slot(&state.export_cancel);

    let app_for_progress = app.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let cancel_for_worker = cancel.clone();
    let options = export_mp4::ExportOptions {
        quality: args.quality.into(),
        resolution: args.resolution.into(),
        has_audio: args.has_audio,
        subtitle_ass: args.subtitle_ass,
    };
    thread::spawn(move || {
        let on_progress = move |p: export_mp4::ExportProgress| {
            let _ = app_for_progress.emit(
                "export-progress",
                ExportProgressEvent {
                    pct: p.pct,
                    message: p.message,
                },
            );
        };
        let res = export_mp4::export(
            &ffmpeg,
            &source,
            &output,
            &args.cutlist,
            options,
            cancel_for_worker,
            on_progress,
        );
        let _ = tx.send(res.map_err(fmt_err));
    });

    let joined = tauri::async_runtime::spawn_blocking(move || {
        rx.recv().unwrap_or_else(|e| Err(e.to_string()))
    })
    .await;
    clear_cancel_slot(&state.export_cancel, &cancel);
    joined.map_err(fmt_err)?
}

#[tauri::command]
pub fn cancel_export(state: State<'_, AppState>) -> Result<(), String> {
    cancel_slot(&state.export_cancel);
    Ok(())
}

#[tauri::command]
pub fn cancel_detect(state: State<'_, AppState>) -> Result<(), String> {
    cancel_slot(&state.detect_cancel);
    Ok(())
}

#[tauri::command]
pub fn cancel_waveform(state: State<'_, AppState>) -> Result<(), String> {
    cancel_slot(&state.waveform_cancel);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ExportFcpxmlArgs {
    pub source: String,
    pub output: String,
    pub cutlist: CutList,
    pub fps: f64,
    pub start_timecode: Option<String>,
    pub title: String,
    #[serde(default = "default_has_audio")]
    pub has_audio: bool,
    /// Extra media cut in lockstep with the reference and emitted as
    /// connected clips on their own lanes. Empty for a single-track export.
    #[serde(default)]
    pub tracks: Vec<TrackArgs>,
}

#[derive(Debug, Deserialize)]
pub struct TrackArgs {
    pub source: String,
    pub name: String,
    pub duration: f64,
    pub start_timecode: Option<String>,
    pub has_video: bool,
    pub has_audio: bool,
    /// Project-time second at which this track's media begins.
    pub offset: f64,
}

/// Reveal a file in the OS file manager. macOS opens Finder with the file
/// selected; Linux opens the parent folder; Windows opens Explorer with
/// the file selected.
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("file does not exist: {path}"));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.args(["-R", &path]);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{path}"));
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let parent = p
            .parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let mut c = std::process::Command::new("xdg-open");
        c.arg(parent);
        c
    };
    cmd.spawn().map_err(fmt_err)?;
    Ok(())
}

/// Open a file with the OS's default handler (e.g. the user's installed
/// video player), instead of trying to play it inside the webview — WebKit
/// on Linux depends on system GStreamer plugins that aren't guaranteed to
/// be installed, so in-app `<video>` playback isn't reliable across setups.
#[tauri::command]
pub fn open_with_default_app(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("file does not exist: {path}"));
    }
    #[allow(deprecated)]
    app.shell().open(&path, None).map_err(fmt_err)
}

#[tauri::command]
pub fn export_fcpxml(args: ExportFcpxmlArgs) -> Result<(), String> {
    let source = PathBuf::from(&args.source);
    let reference = Track {
        path: Some(&source),
        name: &args.title,
        duration: args.cutlist.source_duration,
        start_timecode: args.start_timecode.as_deref(),
        has_video: true,
        has_audio: args.has_audio,
        offset: 0.0,
    };

    let linked_paths: Vec<PathBuf> = args
        .tracks
        .iter()
        .map(|t| PathBuf::from(&t.source))
        .collect();
    let linked: Vec<Track<'_>> = args
        .tracks
        .iter()
        .zip(linked_paths.iter())
        .map(|(t, path)| Track {
            path: Some(path),
            name: &t.name,
            duration: t.duration,
            start_timecode: t.start_timecode.as_deref(),
            has_video: t.has_video,
            has_audio: t.has_audio,
            offset: t.offset,
        })
        .collect();

    let xml = render_fcpxml(
        &args.cutlist,
        FcpxmlParams {
            fps: args.fps,
            title: &args.title,
            reference,
            linked: &linked,
        },
    )
    .map_err(fmt_err)?;
    std::fs::write(&args.output, xml).map_err(fmt_err)?;
    Ok(())
}

// ===================== Batch queue (primary UI) =====================

#[tauri::command]
pub fn queue_add(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Vec<Job> {
    let added: Vec<Job> = paths.into_iter().map(|p| state.queue.add(p)).collect();
    let _ = app.emit("queue-updated", state.queue.snapshot());
    added
}

#[tauri::command]
pub fn queue_snapshot(state: State<'_, AppState>) -> Vec<Job> {
    state.queue.snapshot()
}

/// Removing a queued/finished job deletes it outright; removing a running
/// one only requests cancellation (see `JobQueue::remove`), so the row has
/// to stay visible until the worker notices and marks it failed — hence
/// re-emitting the snapshot here rather than letting the frontend assume
/// the row is already gone.
#[tauri::command]
pub fn queue_remove(app: AppHandle, state: State<'_, AppState>, id: u64) {
    state.queue.remove(id);
    let _ = app.emit("queue-updated", state.queue.snapshot());
}

#[tauri::command]
pub fn queue_retry(app: AppHandle, state: State<'_, AppState>, id: u64) {
    state.queue.retry(id);
    let _ = app.emit("queue-updated", state.queue.snapshot());
}

#[tauri::command]
pub fn queue_clear_done(app: AppHandle, state: State<'_, AppState>) {
    state.queue.clear_done();
    let _ = app.emit("queue-updated", state.queue.snapshot());
}

/// Kick off the worker pool. Safe to call again while a previous run is
/// still finishing up — an idle worker that finds nothing queued exits
/// immediately, so there's never more than `effective_concurrency()`
/// workers actually processing at once.
#[tauri::command]
pub fn queue_start(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let ffmpeg = ffmpeg_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let ffprobe = ffprobe_path(resource_dir(&app).as_deref()).map_err(fmt_err)?;
    let model = model_path(resource_dir(&app).as_deref()).ok();
    let settings = lock(&state.settings).clone();
    let access_token = if settings.llm.auto_rewrite {
        load_active_llm_token().ok().flatten()
    } else {
        None
    };
    crate::queue::start(
        app,
        state.queue.clone(),
        state.app_data_dir.clone(),
        crate::queue::PipelineContext {
            ffmpeg,
            ffprobe,
            model,
            settings,
            access_token,
        },
    );
    Ok(())
}

// ===================== Settings =====================

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    lock(&state.settings).clone()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<(), String> {
    settings::save(&state.app_data_dir, &settings).map_err(fmt_err)?;
    *lock(&state.settings) = settings;
    Ok(())
}

// ===================== Log =====================

#[tauri::command]
pub fn log_list(state: State<'_, AppState>) -> Result<Vec<logstore::LogEntry>, String> {
    logstore::list(&state.app_data_dir).map_err(fmt_err)
}

// ===================== LLM connection (OAuth or plain API key) =====================

/// The bearer token to actually send with an LLM call, picking whichever
/// auth mode is active. An API key always wins over a stored OAuth token —
/// if the user has pasted a key for a custom provider, that's the explicit,
/// most-recent choice; a stale OAuth token from a previously-configured
/// provider shouldn't silently take over.
fn load_active_llm_token() -> Result<Option<String>, String> {
    if let Some(key) = oauth::load_api_key().map_err(fmt_err)? {
        if !key.is_empty() {
            return Ok(Some(key));
        }
    }
    Ok(oauth::load_token()
        .map_err(fmt_err)?
        .map(|t| t.access_token))
}

#[derive(Debug, Serialize)]
pub struct LlmStatus {
    pub connected: bool,
    pub provider_label: String,
    /// "api_key", "oauth", or "none" — which auth mode is actually active,
    /// so the Settings UI can show the matching card instead of guessing.
    pub auth_mode: String,
}

#[tauri::command]
pub fn llm_status(state: State<'_, AppState>) -> LlmStatus {
    let provider_label = lock(&state.settings).llm.provider_label.clone();
    let has_api_key = oauth::load_api_key()
        .ok()
        .flatten()
        .is_some_and(|k| !k.is_empty());
    if has_api_key {
        return LlmStatus {
            connected: true,
            provider_label,
            auth_mode: "api_key".to_string(),
        };
    }
    if oauth::load_token().ok().flatten().is_some() {
        return LlmStatus {
            connected: true,
            provider_label,
            auth_mode: "oauth".to_string(),
        };
    }
    LlmStatus {
        connected: false,
        provider_label,
        auth_mode: "none".to_string(),
    }
}

/// Store a plain bearer API key for a custom OpenAI-compatible provider —
/// the non-OAuth path. `api_base`/`model` from Settings still apply.
#[tauri::command]
pub fn llm_set_api_key(key: String) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("API key kosong".to_string());
    }
    oauth::store_api_key(trimmed).map_err(fmt_err)
}

/// List models the given (or already-stored) credential can use. `token` is
/// the draft API key field in Settings, used before the user has pressed
/// "Simpan & Hubungkan" — once a key/OAuth token is already stored, an empty
/// `token` falls back to it, so the picker also works from the connected view.
#[tauri::command]
pub fn llm_list_models(api_base: String, token: Option<String>) -> Result<Vec<String>, String> {
    let token = match token.map(|t| t.trim().to_string()) {
        Some(t) if !t.is_empty() => t,
        _ => load_active_llm_token()?.ok_or_else(|| "belum ada API key/token".to_string())?,
    };
    llm::list_models(&api_base, &token).map_err(fmt_err)
}

/// Run the full PKCE flow against whatever provider the user configured in
/// Settings: open their system browser at the authorize URL, wait for the
/// loopback redirect, exchange the code, and store the token in the OS
/// keychain. Blocks the calling async task (not the UI) until the user
/// finishes in the browser, fails, or calls `llm_cancel_connect`.
#[tauri::command]
pub async fn llm_connect(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let llm_settings = lock(&state.settings).llm.clone();
    let cancel = install_cancel_slot(&state.llm_cancel);
    let cancel_for_worker = cancel.clone();

    let (verifier, challenge) = oauth::generate_pkce();
    let oauth_state = oauth::random_state();
    let url = oauth::authorize_url(&llm_settings, &oauth_state, &challenge).map_err(fmt_err)?;

    // tauri-plugin-shell's open() is deprecated in favor of a separate
    // tauri-plugin-opener crate; the frontend already depends on this same
    // plugin for its own external-link opens (Dropzone.svelte), so adding a
    // second plugin just for this one call isn't worth the extra surface.
    #[allow(deprecated)]
    app.shell().open(&url, None).map_err(fmt_err)?;

    let joined = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let code = oauth::await_redirect(&oauth_state, cancel_for_worker).map_err(fmt_err)?;
        let token = oauth::exchange_code(&llm_settings, &code, &verifier).map_err(fmt_err)?;
        oauth::store_token(&token).map_err(fmt_err)
    })
    .await;
    clear_cancel_slot(&state.llm_cancel, &cancel);
    joined.map_err(fmt_err)?
}

#[tauri::command]
pub fn llm_cancel_connect(state: State<'_, AppState>) -> Result<(), String> {
    cancel_slot(&state.llm_cancel);
    Ok(())
}

/// Clears both auth modes — whichever is actually set just goes away, the
/// other call is a no-op (`delete_*` already treats a missing entry as
/// success), so this is safe to call regardless of which one is active.
#[tauri::command]
pub fn llm_disconnect() -> Result<(), String> {
    oauth::delete_token().map_err(fmt_err)?;
    oauth::delete_api_key().map_err(fmt_err)
}

/// One-off manual check: send a short sample through the configured LLM so
/// Settings can confirm the connection actually works, without waiting for
/// a full batch job.
#[tauri::command]
pub fn llm_test(state: State<'_, AppState>) -> Result<(), String> {
    let llm_settings = lock(&state.settings).llm.clone();
    let token = load_active_llm_token()?.ok_or_else(|| "belum terhubung ke LLM".to_string())?;
    let sample = [crate::timeline::TimedText::new(0.0, 1.0, "tes transkrip")];
    llm::rewrite_transcript(&llm_settings, &token, &sample).map_err(fmt_err)?;
    Ok(())
}
