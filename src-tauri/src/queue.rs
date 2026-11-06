//! Batch job queue: the primary UI drops N videos in, each runs the same
//! fixed pipeline (detect silence with default params, optionally transcribe
//! and LLM-rewrite and burn in subtitles, then export) with no per-video
//! manual tuning. `queue_start` spawns `settings.effective_concurrency()`
//! worker threads that pull from the shared queue until it's empty — that's
//! the whole "parallel" story; each worker is otherwise a serial pipeline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::audio::extract_pcm_with_progress;
use crate::cutlist::CutList;
use crate::export_mp4::{self, ExportOptions};
use crate::llm::rewrite_transcript;
use crate::logstore::{self, LogEntry};
use crate::probe::probe;
use crate::progress::{fmt_clock, Progress, Stage, StageSpec};
use crate::settings::AppSettings;
use crate::subtitle::{render_ass, SubtitleStyle, VideoSize};
use crate::sync::lock;
use crate::timeline::map_to_output;
use crate::transcribe::transcribe;
use crate::vad::{score_chunks_with_progress, segments_from_scores, VadParams};

const DEFAULT_PAD_SECONDS: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub state: JobState,
    pub pct: f32,
    pub message: String,
    pub output_path: Option<String>,
    pub error: Option<String>,
    /// Per-stage progress of the current or last run; empty while queued.
    pub stages: Vec<Stage>,
}

#[derive(Default)]
pub struct JobQueue {
    jobs: Mutex<Vec<Job>>,
    next_id: AtomicU64,
    cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    /// How many worker threads are alive right now. `start` tops this up to
    /// the configured concurrency instead of always adding more — without
    /// this, clicking "proses semua" again (or adding/retrying a job) while
    /// a previous run's worker is still mid-job spawns an *extra* worker on
    /// top of it, so two jobs end up running at once even though
    /// `effective_concurrency()` says 1. Whisper/VAD aren't built to share a
    /// process under that kind of CPU oversubscription — the observed
    /// symptom was an intermittent "failed to encode" from whisper.cpp that
    /// always "fixed itself" on retry, because the retry usually landed
    /// without a second worker still in flight.
    active_workers: Mutex<usize>,
}

impl JobQueue {
    pub fn add(&self, path: String) -> Job {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let job = Job {
            id,
            name,
            path,
            state: JobState::Queued,
            pct: 0.0,
            message: "menunggu".to_string(),
            output_path: None,
            error: None,
            stages: Vec::new(),
        };
        lock(&self.jobs).push(job.clone());
        job
    }

    pub fn snapshot(&self) -> Vec<Job> {
        lock(&self.jobs).clone()
    }

    /// Deletes a queued/finished job outright. A running job is cancelled
    /// instead — yanking its row out from under the worker would just leave
    /// ffmpeg/whisper burning CPU for a result nobody can see anymore.
    pub fn remove(&self, id: u64) {
        let mut jobs = lock(&self.jobs);
        if let Some(pos) = jobs.iter().position(|j| j.id == id) {
            if jobs[pos].state == JobState::Running {
                drop(jobs);
                self.cancel(id);
                return;
            }
            jobs.remove(pos);
        }
    }

    pub fn clear_done(&self) {
        lock(&self.jobs).retain(|j| j.state != JobState::Done);
    }

    pub fn retry(&self, id: u64) {
        let mut jobs = lock(&self.jobs);
        if let Some(j) = jobs.iter_mut().find(|j| j.id == id) {
            if j.state == JobState::Failed {
                j.state = JobState::Queued;
                j.pct = 0.0;
                j.message = "menunggu".to_string();
                j.error = None;
                j.stages.clear();
            }
        }
    }

    pub fn cancel(&self, id: u64) {
        if let Some(flag) = lock(&self.cancels).get(&id) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn claim_next(&self) -> Option<Job> {
        let mut jobs = lock(&self.jobs);
        let next = jobs.iter_mut().find(|j| j.state == JobState::Queued)?;
        next.state = JobState::Running;
        next.message = "memulai".to_string();
        Some(next.clone())
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Job)) {
        let mut jobs = lock(&self.jobs);
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            f(job);
        }
    }

    fn register_cancel(&self, id: u64, flag: Arc<AtomicBool>) {
        lock(&self.cancels).insert(id, flag);
    }

    fn unregister_cancel(&self, id: u64) {
        lock(&self.cancels).remove(&id);
    }

    /// Claim however many new worker slots are needed to bring the active
    /// count up to `target`, and reserve them atomically so two concurrent
    /// callers (e.g. two quick clicks) can't both think they need to spawn.
    fn reserve_workers(&self, target: usize) -> usize {
        let mut active = lock(&self.active_workers);
        let to_spawn = target.saturating_sub(*active);
        *active += to_spawn;
        to_spawn
    }

    fn release_worker(&self) {
        let mut active = lock(&self.active_workers);
        *active = active.saturating_sub(1);
    }
}

/// Everything a worker needs besides the job itself: resolved binary/model
/// paths, the settings snapshot the run started with, and the LLM token if
/// one was loaded. Bundled so `start`/`worker_loop`/`process_one` don't pile
/// up into a wall of positional arguments.
#[derive(Clone)]
pub struct PipelineContext {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub model: Option<PathBuf>,
    pub settings: AppSettings,
    pub access_token: Option<String>,
}

/// Top up the worker pool to the configured concurrency; each worker pulls
/// jobs off the shared queue until none remain, then exits. Safe to call
/// again while workers from a previous call are still active — this only
/// spawns the shortfall (`target - already active`), so it never ends up
/// running more workers at once than `effective_concurrency()` allows, no
/// matter how many times or how close together it's called.
pub fn start(app: AppHandle, queue: Arc<JobQueue>, app_data_dir: PathBuf, ctx: PipelineContext) {
    let target = ctx.settings.effective_concurrency();
    let to_spawn = queue.reserve_workers(target);
    for _ in 0..to_spawn {
        let app = app.clone();
        let queue = queue.clone();
        let queue_for_release = queue.clone();
        let app_data_dir = app_data_dir.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            worker_loop(app, queue, app_data_dir, ctx);
            queue_for_release.release_worker();
        });
    }
}

fn worker_loop(app: AppHandle, queue: Arc<JobQueue>, app_data_dir: PathBuf, ctx: PipelineContext) {
    while let Some(job) = queue.claim_next() {
        emit_snapshot(&app, &queue);

        let cancel = Arc::new(AtomicBool::new(false));
        queue.register_cancel(job.id, cancel.clone());

        let job_id = job.id;
        let queue_for_progress = queue.clone();
        let app_for_progress = app.clone();
        let progress = Progress::new(
            &stage_specs(ctx.settings.burn_subtitles, has_llm(&ctx)),
            Arc::new(move |stages: &[Stage], overall: f32, message: &str| {
                queue_for_progress.update(job_id, |j| {
                    j.pct = overall;
                    j.message = message.to_string();
                    j.stages = stages.to_vec();
                });
                emit_snapshot(&app_for_progress, &queue_for_progress);
            }),
        );

        let started = std::time::Instant::now();
        let source = PathBuf::from(&job.path);
        let outcome = process_one(&ctx, &source, cancel.clone(), &progress);
        queue.unregister_cancel(job.id);

        let (success, output_path, error, failed_stage) = match &outcome {
            Ok(path) => {
                progress.finish();
                (true, Some(path.display().to_string()), None, None)
            }
            Err(e) => {
                let stage = progress.fail();
                let msg = format!("{e:#}");
                let shown = match &stage {
                    Some(label) => format!("gagal di tahap {label}: {msg}"),
                    None => msg,
                };
                (false, None, Some(shown), stage)
            }
        };
        let stages = progress.stages();
        queue.update(job.id, |j| {
            j.state = if success {
                JobState::Done
            } else {
                JobState::Failed
            };
            j.pct = if success { 100.0 } else { j.pct };
            j.message = if success {
                "selesai".to_string()
            } else {
                error.clone().unwrap_or_else(|| "gagal".to_string())
            };
            j.output_path = output_path.clone();
            j.error = error.clone();
            j.stages = stages.clone();
        });
        emit_snapshot(&app, &queue);

        let finished_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let _ = logstore::append(
            &app_data_dir,
            LogEntry {
                name: job.name.clone(),
                finished_at_ms,
                success,
                output_path,
                error,
                subtitles: ctx.settings.burn_subtitles,
                source_path: Some(job.path.clone()),
                duration_secs: Some(started.elapsed().as_secs_f64()),
                failed_stage,
                stages,
                app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                os: Some(format!(
                    "{} {}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )),
            },
        );
    }
}

fn emit_snapshot(app: &AppHandle, queue: &JobQueue) {
    let _ = app.emit("queue-updated", queue.snapshot());
}

fn has_llm(ctx: &PipelineContext) -> bool {
    ctx.settings.burn_subtitles && ctx.settings.llm.auto_rewrite && ctx.access_token.is_some()
}

/// The stages a run will go through, given its settings. Weights are rough
/// shares of wall time on a CPU-only laptop: transcription dominates.
fn stage_specs(subtitles: bool, llm: bool) -> Vec<StageSpec> {
    let mut v: Vec<StageSpec> = vec![
        ("probe", "Membaca info video", 1.0),
        ("extract", "Mengekstrak audio", 6.0),
        ("detect", "Mendeteksi keheningan", 8.0),
    ];
    if subtitles {
        v.push(("transcribe", "Mentranskrip ucapan", 60.0));
        if llm {
            v.push(("llm", "Merapikan transkrip (LLM)", 5.0));
        }
    }
    v.push((
        "export",
        "Mengekspor MP4",
        if subtitles { 25.0 } else { 40.0 },
    ));
    v
}

/// Run the fixed pipeline for one video, reporting each stage to `progress`.
fn process_one(
    ctx: &PipelineContext,
    source: &Path,
    cancel: Arc<AtomicBool>,
    progress: &Progress,
) -> Result<PathBuf> {
    let PipelineContext {
        ffmpeg,
        ffprobe,
        model,
        settings,
        access_token,
    } = ctx;
    let model = model.as_deref();
    let access_token = access_token.as_deref();

    if cancel.load(Ordering::SeqCst) {
        return Err(anyhow!("dibatalkan"));
    }

    progress.begin("probe");
    let info = probe(ffprobe, source).map_err(|e| anyhow!("{e:#}"))?;
    if !info.has_video {
        return Err(anyhow!("file ini tidak punya gambar video"));
    }
    if !info.has_audio {
        return Err(anyhow!(
            "video ini tidak punya audio — tidak ada keheningan untuk dideteksi"
        ));
    }
    progress.set_detail(
        "probe",
        format!(
            "{}x{} · {:.0} fps · {}",
            info.width,
            info.height,
            info.fps,
            fmt_clock(info.duration)
        ),
    );
    let total = info.duration.max(0.001);

    progress.begin("extract");
    let samples = extract_pcm_with_progress(ffmpeg, source, Some(cancel.clone()), &mut |secs| {
        progress.update(
            (secs / total * 100.0) as f32,
            format!("audio {} / {}", fmt_clock(secs), fmt_clock(total)),
        )
    })
    .map_err(|e| anyhow!("{e:#}"))?;

    progress.begin("detect");
    let vad_params = VadParams::default();
    let scores = score_chunks_with_progress(&samples, Some(cancel.as_ref()), &mut |frac| {
        progress.update(
            frac * 100.0,
            format!(
                "audio {} / {}",
                fmt_clock(frac as f64 * total),
                fmt_clock(total)
            ),
        )
    })
    .map_err(|e| anyhow!("{e:#}"))?;
    let segments = segments_from_scores(&scores, vad_params, 0.0);
    let cutlist = CutList::from_speech_segments(&segments, info.duration, DEFAULT_PAD_SECONDS);
    cutlist.ensure_exportable().map_err(|e| anyhow!("{e:#}"))?;
    let kept = cutlist.total_kept_duration();
    progress.set_detail(
        "detect",
        format!(
            "{} bagian bicara · sisa {} dari {}",
            segments.len(),
            fmt_clock(kept),
            fmt_clock(total)
        ),
    );

    let mut subtitle_ass: Option<String> = None;
    if settings.burn_subtitles {
        let model = model.ok_or_else(|| anyhow!("model whisper belum tersedia"))?;
        progress.begin("transcribe");
        progress.update(0.0, "memuat model");
        let progress_for_whisper = progress.clone();
        let raw_segments = transcribe(model, &samples, cancel.clone(), move |pct| {
            let pct = pct.clamp(0, 100) as f64;
            progress_for_whisper.update(
                pct as f32,
                format!(
                    "audio {} / {}",
                    fmt_clock(pct / 100.0 * total),
                    fmt_clock(total)
                ),
            );
        })
        .map_err(|e| anyhow!("{e:#}"))?;
        progress.set_detail("transcribe", format!("{} segmen teks", raw_segments.len()));
        let mut mapped = map_to_output(&cutlist, &raw_segments);

        if settings.llm.auto_rewrite {
            if let Some(token) = access_token {
                progress.begin("llm");
                // Best-effort: a failed rewrite keeps whisper's own text
                // rather than aborting an otherwise-successful export.
                match rewrite_transcript(&settings.llm, token, &mapped) {
                    Ok(rewritten) => mapped = rewritten,
                    Err(e) => progress.set_detail("llm", format!("dilewati: {e:#}")),
                }
            }
        }

        let style = SubtitleStyle::default();
        let video = VideoSize {
            width: info.width.max(2),
            height: info.height.max(2),
        };
        subtitle_ass = Some(render_ass(&mapped, video, &style));
    }

    progress.begin("export");
    let output = default_output_path(source, settings.output_dir.as_deref());
    let options = ExportOptions {
        quality: settings.quality.into(),
        resolution: settings.resolution.into(),
        has_audio: info.has_audio,
        subtitle_ass,
    };
    let progress_for_export = progress.clone();
    export_mp4::export(
        ffmpeg,
        source,
        &output,
        &cutlist,
        options,
        cancel,
        move |p: export_mp4::ExportProgress| {
            let done = p.pct as f64 / 100.0 * kept;
            progress_for_export.update(
                p.pct,
                format!("video {} / {}", fmt_clock(done), fmt_clock(kept)),
            );
        },
    )
    .map_err(|e| anyhow!("{e:#}"))?;
    progress.set_detail("export", output.display().to_string());

    Ok(output)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// `<dir>/<stem>_autocut.mp4`, numbering upward if that name is already
/// taken — a batch run over similarly-named clips must not silently
/// overwrite an earlier result in the same folder.
fn default_output_path(source: &Path, output_dir: Option<&str>) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());
    let dir = match output_dir {
        Some(d) if !d.trim().is_empty() => expand_tilde(d),
        _ => source
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let mut candidate = dir.join(format!("{stem}_autocut.mp4"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}_autocut_{n}.mp4"));
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_jobs_assigns_increasing_ids() {
        let q = JobQueue::default();
        let a = q.add("/tmp/a.mp4".to_string());
        let b = q.add("/tmp/b.mp4".to_string());
        assert!(b.id > a.id);
        assert_eq!(a.name, "a.mp4");
    }

    #[test]
    fn claim_next_only_takes_queued_jobs() {
        let q = JobQueue::default();
        q.add("/tmp/a.mp4".to_string());
        let claimed = q.claim_next().unwrap();
        assert_eq!(claimed.state, JobState::Running);
        assert!(q.claim_next().is_none());
    }

    #[test]
    fn retry_only_affects_failed_jobs() {
        let q = JobQueue::default();
        let job = q.add("/tmp/a.mp4".to_string());
        q.retry(job.id); // still queued, no-op
        assert_eq!(q.snapshot()[0].state, JobState::Queued);
        q.update(job.id, |j| j.state = JobState::Failed);
        q.retry(job.id);
        assert_eq!(q.snapshot()[0].state, JobState::Queued);
    }

    #[test]
    fn clear_done_keeps_everything_else() {
        let q = JobQueue::default();
        let a = q.add("/tmp/a.mp4".to_string());
        let b = q.add("/tmp/b.mp4".to_string());
        q.update(a.id, |j| j.state = JobState::Done);
        q.clear_done();
        let remaining = q.snapshot();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, b.id);
    }

    #[test]
    fn removing_a_running_job_cancels_instead_of_deleting() {
        let q = JobQueue::default();
        let job = q.add("/tmp/a.mp4".to_string());
        q.update(job.id, |j| j.state = JobState::Running);
        let flag = Arc::new(AtomicBool::new(false));
        q.register_cancel(job.id, flag.clone());
        q.remove(job.id);
        assert!(flag.load(Ordering::SeqCst));
        assert_eq!(
            q.snapshot().len(),
            1,
            "running job row must stay until it finishes"
        );
    }

    #[test]
    fn default_output_path_numbers_up_on_collision() {
        let dir = std::env::temp_dir().join(format!("autocut-queue-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("clip.mp4");
        std::fs::write(&source, b"").unwrap();

        let first = default_output_path(&source, None);
        assert_eq!(first, dir.join("clip_autocut.mp4"));
        std::fs::write(&first, b"").unwrap();

        let second = default_output_path(&source, None);
        assert_eq!(second, dir.join("clip_autocut_2.mp4"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_output_path_honors_configured_output_dir() {
        let dir =
            std::env::temp_dir().join(format!("autocut-queue-test-outdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let source = PathBuf::from("/some/where/clip.mov");
        let out = default_output_path(&source, Some(dir.to_str().unwrap()));
        assert_eq!(out, dir.join("clip_autocut.mp4"));
    }

    #[test]
    fn reserve_workers_only_spawns_the_shortfall() {
        let q = JobQueue::default();
        assert_eq!(q.reserve_workers(1), 1, "first call starts the one worker");
        assert_eq!(
            q.reserve_workers(1),
            0,
            "a second call while that worker is still active must not stack another one"
        );
        q.release_worker();
        assert_eq!(
            q.reserve_workers(1),
            1,
            "once the worker exits, the next call can start a fresh one"
        );
    }

    #[test]
    fn reserve_workers_tops_up_to_a_higher_target() {
        let q = JobQueue::default();
        assert_eq!(q.reserve_workers(1), 1);
        assert_eq!(
            q.reserve_workers(3),
            2,
            "raising concurrency only spawns the extra slots needed"
        );
    }
}
