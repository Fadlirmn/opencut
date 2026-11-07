//! Tighten VAD speech segments to where the voice actually starts and stops.
//!
//! Silero scores 32ms chunks, and its hysteresis holds "speech" a little past
//! the last word, so its edges are coarse. The queue used to make up for that
//! with a fixed 0.3s pad on each side. That never clipped a word, but every
//! pause kept up to 0.6s of dead air, and the cuts felt loose.
//!
//! This pass works at 10ms resolution on the same 16kHz PCM:
//! 1. Measure each 10ms frame's loudness and pick a "voice" threshold just
//!    above this recording's own noise floor (so a noisy room and a quiet
//!    studio both work).
//! 2. From each VAD edge, walk outward (at most `search` seconds) until the
//!    audio has been quiet for `quiet_run`. That quiet run is where the
//!    voice really ended (or before it, began).
//! 3. Keep a short `lead` before the onset and a longer `tail` after the
//!    offset. Consonants like "s"/"h" and word endings decay softly, and
//!    cutting them dead sounds clipped.
//! 4. Merge keeps whose gap is shorter than `min_cut`, so the export never
//!    makes a jump cut too short to notice except as a stutter.
//!
//! Edges also move inward (at most `trim_in`) when the audio right at the VAD
//! edge is already quiet. Silero's hysteresis keeps "speech" on for a while
//! after the last word, and that dead air is exactly what made cuts feel
//! loose. If the voice is still loud at the outward search limit, the edge
//! goes out to the limit (same as the old pad).

use crate::cutlist::SpeechSegment;

#[derive(Debug, Clone, Copy)]
pub struct RefineParams {
    /// Kept before the detected onset.
    pub lead: f64,
    /// Kept after the detected offset.
    pub tail: f64,
    /// Furthest an edge may move outward from the VAD edge.
    pub search: f64,
    /// Furthest an edge may move inward when the VAD edge sits in silence.
    pub trim_in: f64,
    /// Quiet needed to call the voice stopped.
    pub quiet_run: f64,
    /// Gaps shorter than this are kept instead of cut.
    pub min_cut: f64,
}

impl Default for RefineParams {
    fn default() -> Self {
        Self {
            lead: 0.06,
            tail: 0.12,
            search: 0.30,
            trim_in: 0.25,
            quiet_run: 0.06,
            min_cut: 0.20,
        }
    }
}

const FRAME_SECONDS: f64 = 0.010;

/// Loudness of each 10ms frame in dBFS.
fn frame_db(samples: &[i16], rate: u32) -> Vec<f32> {
    let frame = ((rate as f64 * FRAME_SECONDS).round() as usize).max(1);
    samples
        .chunks(frame)
        .map(|c| {
            let sum: f64 = c.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
            let rms = (sum / c.len() as f64).sqrt();
            (20.0 * rms.max(1e-6).log10()) as f32
        })
        .collect()
}

fn percentile(values: &[f32], p: f64) -> f32 {
    if values.is_empty() {
        return -90.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// Level above which a frame counts as voice: 12dB over the noise floor
/// (10th percentile), but never more than 18dB under typical speech (median of
/// the VAD's speech frames), so a loud room doesn't swallow quiet syllables.
fn voice_threshold(db: &[f32], segments: &[SpeechSegment], time_offset: f64) -> f32 {
    let floor = percentile(db, 0.10);
    let speech: Vec<f32> = segments
        .iter()
        .flat_map(|s| {
            let a = to_frame(s.start - time_offset).min(db.len());
            let b = to_frame(s.end - time_offset).min(db.len());
            db[a..b.max(a)].iter().copied()
        })
        .collect();
    let speech_level = percentile(&speech, 0.5);
    (floor + 12.0).min(speech_level - 18.0).max(floor + 3.0)
}

fn to_frame(t: f64) -> usize {
    (t.max(0.0) / FRAME_SECONDS).floor() as usize
}

/// Refine `segments` (in seconds, `time_offset` = where `samples` starts) and
/// return keep ranges ready for `CutList::from_speech_segments` with no pad.
pub fn refine_segments(
    samples: &[i16],
    rate: u32,
    segments: &[SpeechSegment],
    time_offset: f64,
    duration: f64,
    params: RefineParams,
) -> Vec<SpeechSegment> {
    if segments.is_empty() {
        return Vec::new();
    }
    let db = frame_db(samples, rate);
    if db.is_empty() {
        return segments.to_vec();
    }
    let threshold = voice_threshold(&db, segments, time_offset);
    let loud = |i: usize| db.get(i).is_some_and(|&d| d >= threshold);
    let search = to_frame(params.search).max(1);
    let quiet_need = to_frame(params.quiet_run).max(1);
    let trim_in = to_frame(params.trim_in);

    let mut out: Vec<SpeechSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        let s = to_frame(seg.start - time_offset);
        let e = to_frame(seg.end - time_offset);

        if e <= s {
            continue;
        }

        // Onset. VAD start already in silence: move in to the first loud
        // frame. Otherwise walk back until `quiet_need` quiet frames in a
        // row; the voice begins right after them.
        let onset = if !loud(s) {
            (s..(s + trim_in).min(e)).find(|&i| loud(i)).unwrap_or(s)
        } else {
            onset_before(s, search, quiet_need, &loud)
        };

        // Offset, mirrored: VAD end trailing in silence → back to just after
        // the last loud frame; otherwise walk forward.
        let offset = if !loud(e - 1) {
            (e.saturating_sub(trim_in).max(s)..e)
                .rev()
                .find(|&i| loud(i))
                .map(|i| i + 1)
                .unwrap_or(e)
        } else {
            offset_after(e, search, quiet_need, db.len(), &loud)
        };

        let start = (time_offset + onset as f64 * FRAME_SECONDS - params.lead)
            .max(seg.start - params.search)
            .max(0.0);
        let end = (time_offset + offset as f64 * FRAME_SECONDS + params.tail)
            .min(seg.end + params.search)
            .min(duration);
        if end <= start {
            continue;
        }
        match out.last_mut() {
            Some(last) if start - last.end < params.min_cut => last.end = last.end.max(end),
            _ => out.push(SpeechSegment { start, end }),
        }
    }
    out
}

/// Walk back from frame `s` (at most `search` frames) until `quiet_need`
/// quiet frames in a row; returns the first frame after that run.
fn onset_before(
    s: usize,
    search: usize,
    quiet_need: usize,
    loud: &impl Fn(usize) -> bool,
) -> usize {
    let limit = s.saturating_sub(search);
    let mut quiet = 0usize;
    let mut i = s;
    while i > limit {
        i -= 1;
        if loud(i) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= quiet_need {
                return i + quiet;
            }
        }
    }
    limit
}

/// Walk forward from frame `e` (at most `search` frames) until `quiet_need`
/// quiet frames in a row; returns the first frame of that run.
fn offset_after(
    e: usize,
    search: usize,
    quiet_need: usize,
    len: usize,
    loud: &impl Fn(usize) -> bool,
) -> usize {
    let limit = (e + search).min(len);
    let mut quiet = 0usize;
    for j in e..limit {
        if loud(j) {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= quiet_need {
                return j + 1 - quiet;
            }
        }
    }
    limit.max(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// Quiet noise with loud tone bursts at the given (start, end) seconds.
    fn audio(total: f64, bursts: &[(f64, f64)]) -> Vec<i16> {
        let n = (total * RATE as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                let noise = ((i * 7919) % 61) as f64 - 30.0; // ~-60dBFS hiss
                let voice = if bursts.iter().any(|&(a, b)| t >= a && t < b) {
                    (t * 220.0 * std::f64::consts::TAU).sin() * 8000.0
                } else {
                    0.0
                };
                (noise + voice) as i16
            })
            .collect()
    }

    fn seg(start: f64, end: f64) -> SpeechSegment {
        SpeechSegment { start, end }
    }

    #[test]
    fn edges_tighten_to_the_voice_plus_lead_and_tail() {
        // Voice 1.00–2.00s; VAD said 0.96–2.20 (late release, coarse chunks).
        let pcm = audio(4.0, &[(1.0, 2.0)]);
        let p = RefineParams::default();
        let out = refine_segments(&pcm, RATE, &[seg(0.96, 2.20)], 0.0, 4.0, p);
        assert_eq!(out.len(), 1);
        // Both edges move in to the real voice: the 0.2s of dead air VAD's
        // late release kept is gone.
        assert!(
            (out[0].start - (1.0 - p.lead)).abs() < 0.015,
            "{:?}",
            out[0]
        );
        assert!((out[0].end - (2.0 + p.tail)).abs() < 0.015, "{:?}", out[0]);
    }

    #[test]
    fn early_voice_is_found_before_a_late_vad_start() {
        // VAD started 120ms late; the walk back must find the real onset.
        let pcm = audio(4.0, &[(1.0, 2.0)]);
        let p = RefineParams::default();
        let out = refine_segments(&pcm, RATE, &[seg(1.12, 2.0)], 0.0, 4.0, p);
        assert!(
            (out[0].start - (1.0 - p.lead)).abs() < 0.015,
            "{:?}",
            out[0]
        );
        assert!((out[0].end - (2.0 + p.tail)).abs() < 0.015, "{:?}", out[0]);
    }

    #[test]
    fn pauses_lose_their_dead_air() {
        // Two phrases with a 0.8s pause. The old fixed 0.3s pad kept 0.6s of
        // that pause; now only lead + tail (0.18s) is left.
        let pcm = audio(5.0, &[(1.0, 2.0), (2.8, 3.8)]);
        let p = RefineParams::default();
        let out = refine_segments(&pcm, RATE, &[seg(1.0, 2.0), seg(2.8, 3.8)], 0.0, 5.0, p);
        assert_eq!(out.len(), 2, "{out:?}");
        let gap_kept = (out[0].end - 2.0) + (2.8 - out[1].start);
        assert!((gap_kept - (p.lead + p.tail)).abs() < 0.03, "{out:?}");
    }

    #[test]
    fn short_gaps_merge_instead_of_stuttering() {
        // 0.3s pause: after lead + tail only ~0.12s would be cut — below
        // min_cut, so the two keeps become one.
        let pcm = audio(4.0, &[(1.0, 2.0), (2.3, 3.0)]);
        let out = refine_segments(
            &pcm,
            RATE,
            &[seg(1.0, 2.0), seg(2.3, 3.0)],
            0.0,
            4.0,
            RefineParams::default(),
        );
        assert_eq!(out.len(), 1, "{out:?}");
    }

    #[test]
    fn loud_audio_past_the_search_limit_falls_back_to_the_old_pad() {
        // Continuous sound around a short VAD region: no quiet run is found,
        // so the edges go out exactly `search` (the old pad behaviour).
        let pcm = audio(6.0, &[(1.0, 3.5)]);
        let p = RefineParams::default();
        let out = refine_segments(&pcm, RATE, &[seg(2.0, 2.6)], 0.0, 6.0, p);
        assert!(
            (out[0].start - (2.0 - p.search)).abs() < 0.015,
            "{:?}",
            out[0]
        );
        assert!(
            (out[0].end - (2.6 + p.search)).abs() < 0.015,
            "{:?}",
            out[0]
        );
    }

    #[test]
    fn edges_clamp_to_the_file() {
        let pcm = audio(2.0, &[(0.0, 2.0)]);
        let out = refine_segments(
            &pcm,
            RATE,
            &[seg(0.1, 1.95)],
            0.0,
            2.0,
            RefineParams::default(),
        );
        assert!(out[0].start >= 0.0 && out[0].end <= 2.0, "{:?}", out[0]);
    }
}
