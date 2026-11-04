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
