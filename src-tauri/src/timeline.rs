//! Map timed text from source time onto the cut timeline.
//!
//! Whisper (and any other transcriber) reports times against the source file.
//! The MP4 export does not contain the removed regions, so a subtitle at source
//! second 12 may land at output second 4 once three silences before it are cut
//! out. Every subtitle must therefore be re-based through the same `CutList`
//! the exporter uses.
//!
//! A segment spanning a cut is split into one piece per kept interval. Without
//! word-level timestamps the text is divided proportionally by how long each
//! piece is, which keeps every word in the output exactly once instead of
//! repeating the whole line on both sides of a cut.

use crate::cutlist::{CutKind, CutList};

/// A line of timed text in either source or output time.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedText {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

impl TimedText {
    pub fn new(start: f64, end: f64, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }

    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// Fragments shorter than this are cut debris, not subtitles: a keep edge
/// slicing through a word can leave a 30ms sliver that would flash on screen.
const MIN_FRAGMENT_SECONDS: f64 = 0.4;

/// Re-base `segments` from source time to output time.
///
/// Output time starts at zero and advances only across kept intervals, which is
/// exactly how the concat-demuxer export lays the file out.
pub fn map_to_output(cutlist: &CutList, segments: &[TimedText]) -> Vec<TimedText> {
    let keeps = keep_windows(cutlist);
    let mut out = Vec::new();

    for segment in segments {
        let mut parts: Vec<(f64, f64)> = Vec::new();
        for &(keep_start, keep_end, _) in &keeps {
            let start = segment.start.max(keep_start);
            let end = segment.end.min(keep_end);
            if end > start {
                parts.push((start, end));
            }
        }
        if parts.is_empty() {
            continue;
        }

        let texts = split_text(&segment.text, &parts);
        for ((start, end), text) in parts.into_iter().zip(texts) {
            if end - start < MIN_FRAGMENT_SECONDS {
                continue;
            }
            let Some((_, _, output_start)) = keeps
                .iter()
                .find(|(keep_start, keep_end, _)| start >= *keep_start && start < *keep_end)
            else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            out.push(TimedText {
                start: output_start + (start - keep_start_of(&keeps, start)),
                end: output_start + (end - keep_start_of(&keeps, start)),
                text: text.to_string(),
            });
        }
    }

    out.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn keep_start_of(keeps: &[(f64, f64, f64)], t: f64) -> f64 {
    keeps
        .iter()
        .find(|(start, end, _)| t >= *start && t < *end)
        .map(|(start, _, _)| *start)
        .unwrap_or(t)
}

/// Silence inserted between kept pieces when they are joined for
/// transcription, so whisper sees a pause instead of two words glued together.
pub const SPEECH_GAP_SECONDS: f64 = 0.3;

/// Audio built from only the kept parts of a track, plus the map back to
/// source time. Transcribing this instead of the whole file skips every
/// stretch the export is going to cut anyway.
pub struct SpeechTrack {
    pub samples: Vec<i16>,
    /// `(track_start, source_start, len)` in seconds, one per kept piece, in
    /// order. Gaps between pieces belong to no piece.
    pieces: Vec<(f64, f64, f64)>,
}

impl SpeechTrack {
    /// Concatenate the kept intervals of `cutlist` out of `samples` (mono PCM
    /// at `rate` Hz), with `SPEECH_GAP_SECONDS` of silence between them.
    pub fn build(samples: &[i16], rate: u32, cutlist: &CutList) -> Self {
        let rate_f = rate as f64;
        let gap = (SPEECH_GAP_SECONDS * rate_f).round() as usize;
        let mut out = Vec::new();
        let mut pieces = Vec::new();
        for cut in cutlist.intervals.iter().filter(|c| c.kind == CutKind::Keep) {
            let a = ((cut.start.max(0.0) * rate_f).round() as usize).min(samples.len());
            let b = ((cut.end.max(0.0) * rate_f).round() as usize).min(samples.len());
            if b <= a {
                continue;
            }
            if !out.is_empty() {
                out.resize(out.len() + gap, 0);
            }
            pieces.push((
                out.len() as f64 / rate_f,
                a as f64 / rate_f,
                (b - a) as f64 / rate_f,
            ));
            out.extend_from_slice(&samples[a..b]);
        }
        Self {
            samples: out,
            pieces,
        }
    }

    pub fn duration(&self) -> f64 {
        self.pieces
            .last()
            .map_or(0.0, |(start, _, len)| start + len)
    }

    /// Source time for a time on this track. A time inside a gap snaps to the
    /// edge of the piece it belongs to: a start snaps forward to the next
    /// piece, an end snaps back to the previous one.
    fn to_source(&self, t: f64, is_end: bool) -> f64 {
        let mut prev_end: Option<f64> = None;
        for &(start, src, len) in &self.pieces {
            if t < start {
                return if is_end { prev_end.unwrap_or(src) } else { src };
            }
            if t <= start + len {
                return src + (t - start);
            }
            prev_end = Some(src + len);
        }
        prev_end.unwrap_or(t)
    }

    /// Re-base segments transcribed from this track onto source time. A
    /// segment that collapses to nothing (entirely inside a gap) is dropped.
    pub fn segments_to_source(&self, segments: &[TimedText]) -> Vec<TimedText> {
        segments
            .iter()
            .filter_map(|s| {
                let start = self.to_source(s.start, false);
                let end = self.to_source(s.end, true);
                (end > start).then(|| TimedText::new(start, end, s.text.clone()))
            })
            .collect()
    }
}

/// Kept windows with their cumulative output offset.
fn keep_windows(cutlist: &CutList) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    let mut cursor = 0.0;
    for cut in cutlist.intervals.iter().filter(|c| c.kind == CutKind::Keep) {
        let duration = cut.duration();
        if duration <= 0.0 {
            continue;
        }
        out.push((cut.start, cut.end, cursor));
        cursor += duration;
    }
    out
}

/// Divide `text` across `parts` proportionally by part duration.
///
/// Splits on whitespace so a fragment never ends with half a word. If there are
/// fewer words than parts, earlier parts get the words and later ones render
/// nothing (the caller drops empty fragments).
fn split_text(text: &str, parts: &[(f64, f64)]) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if parts.len() <= 1 || words.is_empty() {
        return parts
            .iter()
            .map(|_| text.trim().to_string())
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
            .map(|(i, t)| if i == 0 { t } else { String::new() })
            .collect();
    }

    let total: f64 = parts.iter().map(|(s, e)| (e - s).max(0.0)).sum();
    if total <= 0.0 {
        let mut out = vec![String::new(); parts.len()];
        out[0] = words.join(" ");
        return out;
    }

    // Cumulative duration boundaries decide how many words each part owns.
    // Largest-remainder keeps the counts summing to the word total.
    let mut raw_counts: Vec<f64> = Vec::with_capacity(parts.len());
    let mut cursor_weight = 0.0;
    let mut previous_count = 0usize;
    for (start, end) in parts {
        cursor_weight += (end - start).max(0.0);
        let target = ((cursor_weight / total) * words.len() as f64).round() as usize;
        let count = target.saturating_sub(previous_count);
        raw_counts.push(count as f64);
        previous_count += count;
    }
    // Rounding can leave words unassigned; give the remainder to the last part.
    let assigned: usize = raw_counts.iter().map(|n| *n as usize).sum();
    let mut counts: Vec<usize> = raw_counts.into_iter().map(|n| n as usize).collect();
    if let Some(last) = counts.last_mut() {
        *last += words.len().saturating_sub(assigned);
    }

    let mut out = Vec::with_capacity(parts.len());
    let mut index = 0usize;
    for count in counts {
        let take = count.min(words.len().saturating_sub(index));
        out.push(words[index..index + take].join(" "));
        index += take;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutlist::Cut;

    #[test]
    fn speech_track_keeps_only_kept_audio_with_gaps() {
        // 10 Hz "audio" so seconds map to 10 samples; value = sample index.
        let samples: Vec<i16> = (0..100).collect();
        let cl = cutlist(
            &[
                (0.0, 1.0, CutKind::Remove),
                (1.0, 3.0, CutKind::Keep),
                (3.0, 6.0, CutKind::Remove),
                (6.0, 7.0, CutKind::Keep),
                (7.0, 10.0, CutKind::Remove),
            ],
            10.0,
        );
        let track = SpeechTrack::build(&samples, 10, &cl);
        // 20 + 3 gap + 10
        assert_eq!(track.samples.len(), 33);
        assert_eq!(track.samples[0], 10);
        assert_eq!(&track.samples[20..23], &[0, 0, 0]);
        assert_eq!(track.samples[23], 60);
        assert!((track.duration() - 3.3).abs() < 1e-9);
    }

    #[test]
    fn speech_track_maps_times_back_to_source() {
        let samples = vec![0i16; 100];
        let cl = cutlist(
            &[
                (1.0, 3.0, CutKind::Keep),
                (3.0, 6.0, CutKind::Remove),
                (6.0, 7.0, CutKind::Keep),
            ],
            10.0,
        );
        let track = SpeechTrack::build(&samples, 10, &cl);
        let back = track.segments_to_source(&[
            TimedText::new(0.5, 1.5, "a"),
            // starts in piece 1, ends in piece 2
            TimedText::new(1.8, 2.8, "b"),
            // entirely in the gap
            TimedText::new(2.05, 2.25, "c"),
        ]);
        assert_eq!(back.len(), 2);
        assert!((back[0].start - 1.5).abs() < 1e-9);
        assert!((back[0].end - 2.5).abs() < 1e-9);
        assert!((back[1].start - 2.8).abs() < 1e-9);
        assert!((back[1].end - 6.5).abs() < 1e-9);
    }

    #[test]
    fn speech_track_round_trips_through_output_mapping() {
        // A line spoken inside a kept piece must land at the same output
        // time whether whisper saw the full track or the speech-only one.
        let samples = vec![0i16; 100];
        let cl = cutlist(
            &[
                (0.0, 2.0, CutKind::Remove),
                (2.0, 4.0, CutKind::Keep),
                (4.0, 8.0, CutKind::Remove),
                (8.0, 10.0, CutKind::Keep),
            ],
            10.0,
        );
        let track = SpeechTrack::build(&samples, 10, &cl);
        // "halo" at source 8.5-9.5 sits at track 2.3+0.5 = 2.8-3.8.
        let src = track.segments_to_source(&[TimedText::new(2.8, 3.8, "halo")]);
        let direct = map_to_output(&cl, &[TimedText::new(8.5, 9.5, "halo")]);
        assert_eq!(map_to_output(&cl, &src), direct);
    }

    #[test]
    fn empty_cutlist_gives_empty_track() {
        let cl = cutlist(&[(0.0, 10.0, CutKind::Remove)], 10.0);
        let track = SpeechTrack::build(&[0i16; 100], 10, &cl);
        assert!(track.samples.is_empty());
        assert_eq!(track.duration(), 0.0);
    }

    fn cutlist(intervals: &[(f64, f64, CutKind)], duration: f64) -> CutList {
        CutList {
            source_duration: duration,
            intervals: intervals
                .iter()
                .map(|(start, end, kind)| Cut {
                    start: *start,
                    end: *end,
                    kind: *kind,
                })
                .collect(),
        }
    }

    fn seg(start: f64, end: f64, text: &str) -> TimedText {
        TimedText::new(start, end, text)
    }

    #[test]
    fn segment_inside_one_keep_shifts_by_removed_duration() {
        let cl = cutlist(
            &[
                (0.0, 2.0, CutKind::Remove),
                (2.0, 5.0, CutKind::Keep),
                (5.0, 10.0, CutKind::Remove),
            ],
            10.0,
        );
        let out = map_to_output(&cl, &[seg(3.0, 4.0, "halo")]);
        assert_eq!(out.len(), 1);
        assert!((out[0].start - 1.0).abs() < 1e-9, "{out:?}");
        assert!((out[0].end - 2.0).abs() < 1e-9, "{out:?}");
        assert_eq!(out[0].text, "halo");
    }

    #[test]
    fn segment_spanning_a_removed_gap_is_split_and_text_kept_once() {
        let cl = cutlist(
            &[
                (0.0, 2.0, CutKind::Keep),
                (2.0, 8.0, CutKind::Remove),
                (8.0, 12.0, CutKind::Keep),
            ],
            12.0,
        );
        let out = map_to_output(&cl, &[seg(1.0, 9.0, "satu dua tiga empat lima enam")]);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!((out[0].start - 1.0).abs() < 1e-9);
        assert!((out[0].end - 2.0).abs() < 1e-9);
        assert!((out[1].start - 2.0).abs() < 1e-9);
        assert!((out[1].end - 3.0).abs() < 1e-9);
        let joined = out
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(joined, "satu dua tiga empat lima enam");
    }

    #[test]
    fn segment_entirely_removed_disappears() {
        let cl = cutlist(
            &[
                (0.0, 2.0, CutKind::Keep),
                (2.0, 8.0, CutKind::Remove),
                (8.0, 12.0, CutKind::Keep),
            ],
            12.0,
        );
        assert!(map_to_output(&cl, &[seg(4.0, 5.0, "hilang")]).is_empty());
    }

    #[test]
    fn tiny_fragment_at_a_cut_edge_is_dropped() {
        let cl = cutlist(
            &[(0.0, 2.0, CutKind::Keep), (2.0, 10.0, CutKind::Remove)],
            10.0,
        );
        let out = map_to_output(&cl, &[seg(1.8, 5.0, "kata")]);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn output_offsets_accumulate_across_keeps() {
        let cl = cutlist(
            &[
                (0.0, 1.0, CutKind::Keep),
                (1.0, 3.0, CutKind::Remove),
                (3.0, 5.0, CutKind::Keep),
                (5.0, 6.0, CutKind::Remove),
                (6.0, 8.0, CutKind::Keep),
            ],
            8.0,
        );
        let out = map_to_output(&cl, &[seg(6.5, 7.5, "akhir"), seg(3.2, 4.0, "tengah")]);
        assert_eq!(out.len(), 2);
        assert!((out[0].start - 1.2).abs() < 1e-9, "{out:?}");
        assert!((out[1].start - 3.5).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn split_text_distributes_words_by_duration() {
        let parts = [(0.0, 1.0), (1.0, 4.0)];
        let texts = split_text("a b c d e", &parts);
        assert_eq!(texts, vec!["a".to_string(), "b c d e".to_string()]);
    }

    #[test]
    fn split_text_with_more_parts_than_words_keeps_every_word() {
        let parts = [(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)];
        let texts = split_text("satu dua", &parts);
        let joined = texts.join(" ");
        assert_eq!(
            joined.split_whitespace().collect::<Vec<_>>(),
            vec!["satu", "dua"]
        );
    }
}
