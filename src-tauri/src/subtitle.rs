//! Subtitle rendering: ASS for burn-in, SRT as the fallback when the bundled
//! ffmpeg was compiled without libass.
//!
//! ASS carries the styling that makes a burned subtitle legible over arbitrary
//! footage (outline, bottom margin, resolution-relative size). The renderer is
//! deliberately small: one default style, two lines, no overrides from the
//! transcript.

use crate::timeline::TimedText;

#[derive(Debug, Clone)]
pub struct SubtitleStyle {
    pub font_name: String,
    /// Point size at the output resolution. `None` means "scale from height".
    pub font_size: Option<u32>,
    pub max_chars_per_line: usize,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_name: "Arial".to_string(),
            font_size: None,
            max_chars_per_line: 42,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VideoSize {
    pub width: u32,
    pub height: u32,
}

/// Render an ASS file. `segments` are already in output time.
pub fn render_ass(segments: &[TimedText], video: VideoSize, style: &SubtitleStyle) -> String {
    let width = video.width.max(2);
    let height = video.height.max(2);
    let font_size = style
        .font_size
        .unwrap_or_else(|| ((height as f64 * 0.045).round() as u32).clamp(16, 64));
    let margin_v = ((height as f64 * 0.04).round() as u32).max(12);
    let margin_h = ((width as f64 * 0.04).round() as u32).max(12);

    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str(&format!("PlayResX: {width}\n"));
    out.push_str(&format!("PlayResY: {height}\n"));
    out.push_str("WrapStyle: 0\n");
    out.push_str("ScaledBorderAndShadow: yes\n");
    out.push_str("YCbCr Matrix: TV.709\n\n");

    out.push_str("[V4+ Styles]\n");
    out.push_str(
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
    );
    out.push_str(&format!(
        "Style: Default,{},{},&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,{margin_h},{margin_h},{margin_v},1\n\n",
        sanitize_font_name(&style.font_name),
        font_size
    ));

    out.push_str("[Events]\n");
    out.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for segment in segments {
        if segment.end <= segment.start || segment.text.trim().is_empty() {
            continue;
        }
        let text = wrap_text(&segment.text, style.max_chars_per_line);
        let text = escape_ass(&text);
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            ass_time(segment.start),
            ass_time(segment.end),
            text
        ));
    }
    out
}

/// Fallback subtitle file for builds whose ffmpeg lacks libass. SRT uses a
/// comma decimal separator and CRLF, which is what players and NLEs expect.
pub fn render_srt(segments: &[TimedText]) -> String {
    let mut out = String::new();
    let mut index = 1usize;
    for segment in segments {
        if segment.end <= segment.start || segment.text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("{index}\r\n"));
        out.push_str(&format!(
            "{} --> {}\r\n",
            srt_time(segment.start),
            srt_time(segment.end)
        ));
        out.push_str(segment.text.trim());
        out.push_str("\r\n\r\n");
        index += 1;
    }
    out
}

/// ASS wants `H:MM:SS.cc` with centisecond precision.
pub fn ass_time(seconds: f64) -> String {
    let total_cs = (seconds.max(0.0) * 100.0).round() as u64;
    let cs = total_cs % 100;
    let total_seconds = total_cs / 100;
    let s = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let m = total_minutes % 60;
    let h = total_minutes / 60;
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

/// SRT wants `HH:MM:SS,mmm`.
pub fn srt_time(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_seconds = total_ms / 1000;
    let s = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let m = total_minutes % 60;
    let h = total_minutes / 60;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

fn sanitize_font_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, ',' | '\n' | '\r'))
        .collect();
    if cleaned.trim().is_empty() {
        "Arial".to_string()
    } else {
        cleaned
    }
}

/// Break text into at most `max_lines` display lines, balancing the second
/// line rather than leaving one orphan word on it.
fn wrap_text(text: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(12);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let joined = words.join(" ");
    if joined.chars().count() <= max_chars {
        return joined;
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in &words {
        let candidate_len = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if candidate_len > max_chars && !current.is_empty() {
            lines.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }

    if lines.len() <= 2 {
        return lines.join("\\N");
    }
    // More than two greedy lines: split at the word closest to half the text
    // so both lines are balanced instead of leaving a long tail.
    let total_chars = joined.chars().count();
    let target = total_chars / 2;
    let mut first: Vec<&str> = Vec::new();
    let mut second: Vec<&str> = Vec::new();
    let mut length = 0usize;
    let mut split_at = 0usize;
    let mut best_delta = usize::MAX;
    for (i, word) in words.iter().enumerate() {
        let candidate = length + word.chars().count() + usize::from(!first.is_empty());
        if candidate.abs_diff(target) < best_delta {
            best_delta = candidate.abs_diff(target);
            split_at = i + 1;
        }
        length = candidate;
    }
    let split_at = split_at.clamp(1, words.len().saturating_sub(1));
    first.extend_from_slice(&words[..split_at]);
    second.extend_from_slice(&words[split_at..]);
    format!("{}\\N{}", first.join(" "), second.join(" "))
}

/// Escape ASS override syntax. A literal backslash stays a backslash; braces
/// become escaped so a transcript saying `{music}` is displayed, not parsed.
fn escape_ass(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\r' => {}
            '\n' => out.push_str("\\N"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TimedText {
        TimedText::new(start, end, text)
    }

    #[test]
    fn ass_time_rounds_to_centiseconds() {
        assert_eq!(ass_time(0.0), "0:00:00.00");
        assert_eq!(ass_time(1.234), "0:00:01.23");
        assert_eq!(ass_time(61.999), "0:01:02.00");
        assert_eq!(ass_time(3661.5), "1:01:01.50");
    }

    #[test]
    fn srt_time_uses_comma_and_milliseconds() {
        assert_eq!(srt_time(0.0), "00:00:00,000");
        assert_eq!(srt_time(1.2345), "00:00:01,235");
        assert_eq!(srt_time(3661.5), "01:01:01,500");
    }

    #[test]
    fn ass_contains_header_resolution_and_dialogue() {
        let ass = render_ass(
            &[seg(1.0, 2.5, "halo dunia")],
            VideoSize {
                width: 1920,
                height: 1080,
            },
            &SubtitleStyle::default(),
        );
        assert!(ass.contains("PlayResX: 1920"), "{ass}");
        assert!(ass.contains("PlayResY: 1080"), "{ass}");
        assert!(
            ass.contains("Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,halo dunia"),
            "{ass}"
        );
    }

    #[test]
    fn ass_escapes_override_braces_and_backslashes() {
        let ass = render_ass(
            &[seg(0.0, 1.0, r"a {b} c\d")],
            VideoSize {
                width: 640,
                height: 360,
            },
            &SubtitleStyle::default(),
        );
        assert!(ass.contains(r"a \{b\} c\\d"), "{ass}");
    }

    #[test]
    fn long_text_wraps_to_two_balanced_lines() {
        let text = "satu dua tiga empat lima enam tujuh delapan sembilan sepuluh sebelas dua belas";
        let wrapped = wrap_text(text, 24);
        assert_eq!(wrapped.matches("\\N").count(), 1, "{wrapped}");
        assert_eq!(
            wrapped
                .replace("\\N", " ")
                .split_whitespace()
                .collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_segments_are_skipped() {
        let ass = render_ass(
            &[
                seg(0.0, 1.0, "   "),
                seg(1.0, 1.0, "zero"),
                seg(2.0, 3.0, "ok"),
            ],
            VideoSize {
                width: 640,
                height: 360,
            },
            &SubtitleStyle::default(),
        );
        assert_eq!(ass.matches("Dialogue:").count(), 1, "{ass}");
    }

    #[test]
    fn srt_numbers_and_separates_entries() {
        let srt = render_srt(&[seg(0.5, 1.5, "satu"), seg(2.0, 3.0, "dua")]);
        assert!(srt.starts_with("1\r\n00:00:00,500 --> 00:00:01,500\r\nsatu\r\n\r\n"));
        assert!(srt.contains("2\r\n00:00:02,000 --> 00:00:03,000\r\ndua"));
    }
}
