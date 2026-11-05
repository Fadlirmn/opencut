//! Whisper transcription: 16kHz mono PCM in, timed text in source time out.
//!
//! Forces the language to Indonesian (`id`) rather than auto-detecting. That
//! is both faster (skips whisper's language-ID pass) and more accurate than
//! detection for audio that is reliably Indonesian, which is this app's
//! target use case.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::to_unit;
use crate::timeline::TimedText;

const LANGUAGE: &str = "id";

/// Run a full transcription pass over `samples` (16kHz mono PCM, the same
/// format `AnalysisCache` keeps) and return timed text in source time.
///
/// `cancel` is checked before loading the model and polled by whisper's abort
/// callback during inference; whisper.cpp has no mid-segment resume, so an
/// aborted run returns an error rather than partial output.
pub fn transcribe(
    model_path: &Path,
    samples: &[i16],
    cancel: Arc<AtomicBool>,
) -> Result<Vec<TimedText>> {
    if is_cancelled(&cancel) {
        return Err(anyhow!("transcription cancelled"));
    }

    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .with_context(|| format!("loading whisper model at {}", model_path.display()))?;
    let mut state = ctx.create_state().context("creating whisper state")?;

    // whisper-rs's `full()` wants one contiguous f32 slice, unlike silero's
    // chunked `predict` — the whole track is converted up front rather than
    // streamed, so this briefly costs an extra samples.len() * 4 bytes.
    let pcm: Vec<f32> = samples.iter().copied().map(to_unit).collect();

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    params.set_n_threads(threads);
    params.set_language(Some(LANGUAGE));
    params.set_translate(false);
    params.set_detect_language(false);
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    let abort_flag = cancel.clone();
    params.set_abort_callback_safe(move || abort_flag.load(Ordering::SeqCst));

    state
        .full(params, &pcm)
        .context("running whisper transcription")?;

    if is_cancelled(&cancel) {
        return Err(anyhow!("transcription cancelled"));
    }

    let mut out = Vec::new();
    for segment in state.as_iter() {
        let text = segment
            .to_str_lossy()
            .context("decoding whisper segment text")?;
        if let Some(timed) =
            segment_to_timed_text(segment.start_timestamp(), segment.end_timestamp(), &text)
        {
            out.push(timed);
        }
    }
    Ok(out)
}

fn is_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

/// Convert one whisper segment's centisecond timestamps + text into
/// `TimedText` (source time). Returns `None` for a degenerate or blank
/// segment rather than letting an empty caption through.
fn segment_to_timed_text(start_cs: i64, end_cs: i64, text: &str) -> Option<TimedText> {
    let start = start_cs.max(0) as f64 / 100.0;
    let end = end_cs.max(0) as f64 / 100.0;
    let text = text.trim();
    if end <= start || text.is_empty() {
        return None;
    }
    Some(TimedText::new(start, end, text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centiseconds_convert_to_seconds() {
        let t = segment_to_timed_text(123, 456, "halo").unwrap();
        assert!((t.start - 1.23).abs() < 1e-9);
        assert!((t.end - 4.56).abs() < 1e-9);
        assert_eq!(t.text, "halo");
    }

    #[test]
    fn blank_segment_text_is_dropped() {
        assert!(segment_to_timed_text(0, 100, "   ").is_none());
    }

    #[test]
    fn zero_duration_segment_is_dropped() {
        assert!(segment_to_timed_text(100, 100, "halo").is_none());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let t = segment_to_timed_text(0, 100, "  halo dunia  ").unwrap();
        assert_eq!(t.text, "halo dunia");
    }
}
