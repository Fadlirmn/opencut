//! Subtitle rendering: ASS for burn-in, SRT as the fallback when the bundled
//! ffmpeg was compiled without libass.
//!
//! ASS carries the styling that makes a burned subtitle legible over arbitrary
//! footage (outline, bottom margin, resolution-relative size). The renderer is
//! deliberately small: one user-configurable style ([`SubtitleStyle`], saved
//! in settings), two lines, no overrides from the transcript.

use serde::{Deserialize, Serialize};

use crate::timeline::TimedText;

/// User-tunable caption look, persisted in settings and mirrored by the
/// preview in the Settings page (`src/lib/subtitle.ts`). Sizes are relative
/// so one style reads the same on a 1080x1920 TikTok clip and a 4K landscape
/// one: anything "relative to the short side" is measured against
/// `min(width, height)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubtitleStyle {
    pub font_name: String,
    /// Glyph size as a percentage of the frame's short side.
    pub size_pct: f64,
    /// Distance from the bottom edge to the bottom of the caption, as a
    /// percentage of the frame height.
    pub position_pct: f64,
    /// `#RRGGBB`.
    pub text_color: String,
    /// `#RRGGBB`. Also the box colour when `background` is on.
    pub outline_color: String,
    /// Outline thickness relative to the short side, in 1/1000ths (so 4.0 on
    /// a 1080-wide portrait clip is ~4px).
    pub outline: f64,
    pub bold: bool,
    pub uppercase: bool,
    /// Draw an opaque-ish box behind each line instead of an outline.
    pub background: bool,
    /// Box opacity in percent, used only with `background`.
    pub background_opacity: u32,
    /// Characters per line before wrapping; 0 picks it from the frame width
    /// and font size so the line never runs past the side margins.
    pub max_chars_per_line: usize,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_name: "Arial".to_string(),
            size_pct: 7.0,
            position_pct: 25.0,
            text_color: "#FFFFFF".to_string(),
            outline_color: "#000000".to_string(),
            outline: 5.0,
            bold: true,
            uppercase: false,
            background: false,
            background_opacity: 60,
            max_chars_per_line: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VideoSize {
    pub width: u32,
    pub height: u32,
}

/// Pixel metrics for a style at a given frame size. Shared by the renderer
/// and its tests; the TS preview computes the same numbers.
struct Metrics {
    font_size: u32,
    outline: f64,
    margin_v: u32,
    margin_h: u32,
    max_chars: usize,
}

/// Rough advance width of an average Latin glyph relative to the font size.
/// Bold Arial averages ~0.55em; regular ~0.5em.
fn char_width_em(bold: bool) -> f64 {
    if bold {
        0.55
    } else {
        0.5
    }
}

fn metrics(video: VideoSize, style: &SubtitleStyle) -> Metrics {
    let width = video.width.max(2) as f64;
    let height = video.height.max(2) as f64;
    let short = width.min(height);
    let size_pct = if style.size_pct.is_finite() {
        style.size_pct.clamp(2.0, 20.0)
    } else {
        7.0
    };
    let font_size = ((short * size_pct / 100.0).round() as u32).max(8);
    let outline = (short * style.outline.clamp(0.0, 30.0) / 1000.0 * 10.0).round() / 10.0;
    let position_pct = if style.position_pct.is_finite() {
        style.position_pct.clamp(0.0, 90.0)
    } else {
        25.0
    };
    let margin_v = (height * position_pct / 100.0).round() as u32;
    let margin_h = ((width * 0.05).round() as u32).max(12);
    let max_chars = if style.max_chars_per_line > 0 {
        style.max_chars_per_line
    } else {
        let usable = width - 2.0 * margin_h as f64;
        ((usable / (font_size as f64 * char_width_em(style.bold))).floor() as usize).clamp(8, 60)
    };
    Metrics {
        font_size,
        outline,
        margin_v,
        margin_h,
        max_chars,
    }
}

/// `#RRGGBB` → ASS `&HAABBGGRR` with the given alpha (00 = opaque).
fn ass_color(hex: &str, alpha: u8, fallback: &str) -> String {
    let parse = |h: &str| -> Option<(u8, u8, u8)> {
        let h = h.trim().strip_prefix('#').unwrap_or(h.trim());
        if h.len() != 6 {
            return None;
        }
        let n = u32::from_str_radix(h, 16).ok()?;
        Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
    };
    let (r, g, b) = parse(hex)
        .or_else(|| parse(fallback))
        .unwrap_or((255, 255, 255));
    format!("&H{alpha:02X}{b:02X}{g:02X}{r:02X}")
}

/// Render an ASS file. `segments` are already in output time.
pub fn render_ass(segments: &[TimedText], video: VideoSize, style: &SubtitleStyle) -> String {
    let width = video.width.max(2);
    let height = video.height.max(2);
    let m = metrics(video, style);

    let primary = ass_color(&style.text_color, 0, "#FFFFFF");
    // BorderStyle 3 draws an opaque box in the outline colour; its alpha is
    // what makes the box see-through.
    let (border_style, outline_colour) = if style.background {
        let opacity = style.background_opacity.min(100) as f64 / 100.0;
        let alpha = ((1.0 - opacity) * 255.0).round() as u8;
        (3, ass_color(&style.outline_color, alpha, "#000000"))
    } else {
        (1, ass_color(&style.outline_color, 0, "#000000"))
    };
    // With a box the "outline" is the box padding; keep a visible minimum.
    let outline = if style.background {
        m.outline.max(m.font_size as f64 * 0.15)
    } else {
        m.outline
    };

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
        "Style: Default,{},{},{primary},&H000000FF,{outline_colour},&H80000000,{},0,0,0,100,100,0,0,{border_style},{outline},0,2,{mh},{mh},{mv},1\n\n",
        sanitize_font_name(&style.font_name),
        m.font_size,
        if style.bold { -1 } else { 0 },
        mh = m.margin_h,
        mv = m.margin_v,
    ));

    out.push_str("[Events]\n");
    out.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for segment in segments {
        if segment.end <= segment.start || segment.text.trim().is_empty() {
            continue;
        }
        let text = if style.uppercase {
            segment.text.to_uppercase()
        } else {
            segment.text.clone()
        };
        let text = wrap_text(&text, m.max_chars);
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

/// Break text into at most two display lines, balancing the second line
/// rather than leaving one orphan word on it. Lines are joined with a plain
/// `\n`; [`escape_ass`] turns that into ASS's `\N`. Emitting `\N` here got
/// its backslash escaped to `\\N`, which libass draws as a stray `\` at the end
/// of the first line.
fn wrap_text(text: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(8);
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
        return lines.join("\n");
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
    format!("{}\n{}", first.join(" "), second.join(" "))
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
        assert_eq!(wrapped.matches('\n').count(), 1, "{wrapped}");
        assert_eq!(
            wrapped
                .replace('\n', " ")
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

    fn style_line(ass: &str) -> Vec<String> {
        ass.lines()
            .find(|l| l.starts_with("Style: Default,"))
            .expect("style line")
            .trim_start_matches("Style: ")
            .split(',')
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn default_size_scales_with_the_short_side() {
        let portrait = render_ass(
            &[],
            VideoSize {
                width: 1080,
                height: 1920,
            },
            &SubtitleStyle::default(),
        );
        let landscape = render_ass(
            &[],
            VideoSize {
                width: 1920,
                height: 1080,
            },
            &SubtitleStyle::default(),
        );
        // 7% of 1080 — same glyph size in both orientations, and larger than
        // the old height-based size capped at 64.
        assert_eq!(style_line(&portrait)[2], "76");
        assert_eq!(style_line(&landscape)[2], "76");
    }

    #[test]
    fn colours_convert_to_ass_bgr_order() {
        assert_eq!(ass_color("#FFCC00", 0, "#FFFFFF"), "&H0000CCFF");
        assert_eq!(ass_color("bogus", 0, "#000000"), "&H00000000");
        assert_eq!(ass_color("#112233", 0x66, "#000000"), "&H66332211");
    }

    #[test]
    fn background_box_uses_border_style_three_with_alpha() {
        let style = SubtitleStyle {
            background: true,
            background_opacity: 60,
            outline_color: "#000000".into(),
            ..Default::default()
        };
        let fields = style_line(&render_ass(
            &[],
            VideoSize {
                width: 1080,
                height: 1920,
            },
            &style,
        ));
        assert_eq!(fields[15], "3", "{fields:?}");
        assert_eq!(fields[5], "&H66000000", "{fields:?}");
    }

    #[test]
    fn position_sets_the_bottom_margin() {
        let style = SubtitleStyle {
            position_pct: 10.0,
            ..Default::default()
        };
        let fields = style_line(&render_ass(
            &[],
            VideoSize {
                width: 1080,
                height: 1920,
            },
            &style,
        ));
        assert_eq!(fields[21], "192");
    }

    #[test]
    fn uppercase_and_auto_wrap_apply_to_dialogue() {
        let style = SubtitleStyle {
            uppercase: true,
            ..Default::default()
        };
        let ass = render_ass(
            &[seg(
                0.0,
                2.0,
                "ini adalah kalimat yang cukup panjang untuk dipecah",
            )],
            VideoSize {
                width: 1080,
                height: 1920,
            },
            &style,
        );
        let line = ass.lines().find(|l| l.starts_with("Dialogue:")).unwrap();
        assert!(line.contains("INI ADALAH"), "{line}");
        // Exactly ASS's line break, not an escaped backslash before it.
        assert!(line.contains("\\N"), "{line}");
        assert!(!line.contains("\\\\N"), "{line}");
    }

    #[test]
    fn old_settings_without_a_style_get_the_defaults() {
        let s: SubtitleStyle = serde_json::from_str(r#"{"size_pct": 9}"#).unwrap();
        assert_eq!(s.size_pct, 9.0);
        assert!(s.bold);
        assert_eq!(s.text_color, "#FFFFFF");
    }

    #[test]
    fn srt_numbers_and_separates_entries() {
        let srt = render_srt(&[seg(0.5, 1.5, "satu"), seg(2.0, 3.0, "dua")]);
        assert!(srt.starts_with("1\r\n00:00:00,500 --> 00:00:01,500\r\nsatu\r\n\r\n"));
        assert!(srt.contains("2\r\n00:00:02,000 --> 00:00:03,000\r\ndua"));
    }
}
