//! Per-stage progress for one pipeline run.
//!
//! A job is a fixed list of stages (probe, extract, detect, transcribe, ...).
//! Each stage carries its own 0-100 progress, a human-readable detail line,
//! elapsed time and an ETA, so the UI can draw one bar per stage instead of a
//! single number that sits still for minutes during transcription. The overall
//! percentage is a weighted sum over the stages, so a long transcription
//! moves the overall bar too.
//!
//! The same `Stage` list is written into the job log, so a failed run records
//! which stage broke and how long each one took.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::sync::lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub key: String,
    pub label: String,
    pub state: StageState,
    /// 0-100 within this stage.
    pub pct: f32,
    /// Free-form detail such as "audio 04:30 / 10:23".
    pub detail: String,
    pub elapsed_secs: f64,
    /// Estimated seconds left in this stage, once there is enough signal.
    #[serde(default)]
    pub eta_secs: Option<f64>,
}

/// `(key, label, weight)`. Weights are rough shares of wall time on a typical
/// laptop and only shape the overall bar; they need not sum to anything.
pub type StageSpec = (&'static str, &'static str, f32);

pub type EmitFn = dyn Fn(&[Stage], f32, &str) + Send + Sync;

struct Inner {
    stages: Vec<Stage>,
    weights: Vec<f32>,
    current: Option<usize>,
    stage_started: Instant,
    last_emit: Option<Instant>,
}

/// Cheap to clone; clones share state, so a reporter can be handed to a
/// callback that runs on another thread (ffmpeg's stderr reader, say).
#[derive(Clone)]
pub struct Progress {
    inner: Arc<Mutex<Inner>>,
    emit: Arc<EmitFn>,
}

/// Minimum gap between emitted updates inside one stage. Stage transitions
/// always emit immediately.
const EMIT_INTERVAL: Duration = Duration::from_millis(200);

impl Progress {
    pub fn new(specs: &[StageSpec], emit: Arc<EmitFn>) -> Self {
        let stages = specs
            .iter()
            .map(|(key, label, _)| Stage {
                key: key.to_string(),
                label: label.to_string(),
                state: StageState::Pending,
                pct: 0.0,
                detail: String::new(),
                elapsed_secs: 0.0,
                eta_secs: None,
            })
            .collect();
        let weights = specs.iter().map(|(_, _, w)| *w).collect();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                stages,
                weights,
                current: None,
                stage_started: Instant::now(),
                last_emit: None,
            })),
            emit,
        }
    }

    /// Mark the stage `key` running. Any stage still running is closed as done.
    pub fn begin(&self, key: &str) {
        {
            let mut g = lock(&self.inner);
            if let Some(i) = g.current {
                close(&mut g, i, StageState::Done);
            }
            let Some(i) = g.stages.iter().position(|s| s.key == key) else {
                return;
            };
            g.current = Some(i);
            g.stage_started = Instant::now();
            let s = &mut g.stages[i];
            s.state = StageState::Running;
            s.pct = 0.0;
            s.detail.clear();
            s.eta_secs = None;
        }
        self.emit_now();
    }

    /// Update the running stage. `pct` is clamped to 0-99.9 so the bar never
    /// reads 100% before the stage has actually finished.
    pub fn update(&self, pct: f32, detail: impl Into<String>) {
        let should_emit = {
            let mut g = lock(&self.inner);
            let Some(i) = g.current else { return };
            let elapsed = g.stage_started.elapsed().as_secs_f64();
            let s = &mut g.stages[i];
            s.pct = pct.clamp(0.0, 99.9);
            s.detail = detail.into();
            s.elapsed_secs = elapsed;
            s.eta_secs = eta(elapsed, s.pct);
            let now = Instant::now();
            let due = g
                .last_emit
                .map_or(true, |t| now.duration_since(t) >= EMIT_INTERVAL);
            if due {
                g.last_emit = Some(now);
            }
            due
        };
        if should_emit {
            self.emit_now();
        }
    }

    /// Set the detail line of a stage after it finished (e.g. probe results).
    pub fn set_detail(&self, key: &str, detail: impl Into<String>) {
        let mut g = lock(&self.inner);
        if let Some(s) = g.stages.iter_mut().find(|s| s.key == key) {
            s.detail = detail.into();
        }
    }

    /// Close the running stage as done.
    pub fn finish(&self) {
        {
            let mut g = lock(&self.inner);
            if let Some(i) = g.current.take() {
                close(&mut g, i, StageState::Done);
            }
        }
        self.emit_now();
    }

    /// Close the running stage as failed and return its label.
    pub fn fail(&self) -> Option<String> {
        let label = {
            let mut g = lock(&self.inner);
            let i = g.current.take()?;
            close(&mut g, i, StageState::Failed);
            g.stages[i].label.clone()
        };
        self.emit_now();
        Some(label)
    }

    pub fn stages(&self) -> Vec<Stage> {
        lock(&self.inner).stages.clone()
    }

    pub fn overall(&self) -> f32 {
        overall(&lock(&self.inner))
    }

    fn emit_now(&self) {
        let (stages, overall, message) = {
            let mut g = lock(&self.inner);
            g.last_emit = Some(Instant::now());
            let message = match g.current {
                Some(i) => {
                    let s = &g.stages[i];
                    if s.detail.is_empty() {
                        s.label.clone()
                    } else {
                        format!("{} · {}", s.label, s.detail)
                    }
                }
                None => String::new(),
            };
            (g.stages.clone(), overall(&g), message)
        };
        (self.emit)(&stages, overall, &message);
    }
}

fn close(g: &mut Inner, i: usize, state: StageState) {
    let elapsed = g.stage_started.elapsed().as_secs_f64();
    let s = &mut g.stages[i];
    s.state = state;
    s.elapsed_secs = elapsed;
    s.eta_secs = None;
    if state == StageState::Done {
        s.pct = 100.0;
    }
}

fn overall(g: &Inner) -> f32 {
    let total: f32 = g.weights.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let done: f32 = g
        .stages
        .iter()
        .zip(&g.weights)
        .map(|(s, w)| w * s.pct / 100.0)
        .sum();
    (done / total * 100.0).clamp(0.0, 100.0)
}

/// Linear extrapolation, only once the stage is far enough along to mean
/// anything: at least 2% done and 3 seconds in.
fn eta(elapsed: f64, pct: f32) -> Option<f64> {
    if pct < 2.0 || elapsed < 3.0 {
        return None;
    }
    let pct = pct as f64;
    Some(elapsed * (100.0 - pct) / pct)
}

/// `mm:ss`, or `h:mm:ss` past an hour.
pub fn fmt_clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tracker(specs: &[StageSpec]) -> (Progress, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let p = Progress::new(
            specs,
            Arc::new(move |_: &[Stage], _: f32, _: &str| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        (p, count)
    }

    #[test]
    fn overall_is_weighted_by_stage() {
        let (p, _) = tracker(&[("a", "A", 1.0), ("b", "B", 3.0)]);
        p.begin("a");
        p.finish();
        assert!((p.overall() - 25.0).abs() < 0.01);
        p.begin("b");
        p.update(50.0, "");
        assert!((p.overall() - 62.5).abs() < 0.01);
    }

    #[test]
    fn begin_closes_previous_stage() {
        let (p, _) = tracker(&[("a", "A", 1.0), ("b", "B", 1.0)]);
        p.begin("a");
        p.begin("b");
        let s = p.stages();
        assert_eq!(s[0].state, StageState::Done);
        assert_eq!(s[0].pct, 100.0);
        assert_eq!(s[1].state, StageState::Running);
    }

    #[test]
    fn fail_marks_running_stage_and_names_it() {
        let (p, _) = tracker(&[("a", "Ekstrak", 1.0), ("b", "Transkrip", 1.0)]);
        p.begin("a");
        p.begin("b");
        assert_eq!(p.fail().as_deref(), Some("Transkrip"));
        assert_eq!(p.stages()[1].state, StageState::Failed);
        assert_eq!(p.fail(), None);
    }

    #[test]
    fn update_never_reports_complete_before_finish() {
        let (p, _) = tracker(&[("a", "A", 1.0)]);
        p.begin("a");
        p.update(100.0, "");
        assert!(p.stages()[0].pct < 100.0);
    }

    #[test]
    fn updates_are_throttled_but_transitions_are_not() {
        let (p, count) = tracker(&[("a", "A", 1.0), ("b", "B", 1.0)]);
        p.begin("a");
        for i in 0..100 {
            p.update(i as f32, "");
        }
        p.begin("b");
        // begin(a) + begin(b); every update landed inside the throttle window.
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn eta_needs_enough_signal() {
        assert_eq!(eta(10.0, 1.0), None);
        assert_eq!(eta(1.0, 50.0), None);
        assert!((eta(10.0, 25.0).unwrap() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn clock_formatting() {
        assert_eq!(fmt_clock(0.0), "00:00");
        assert_eq!(fmt_clock(75.4), "01:15");
        assert_eq!(fmt_clock(3725.0), "1:02:05");
    }
}
