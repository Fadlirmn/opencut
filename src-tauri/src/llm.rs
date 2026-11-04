//! Send a whisper transcript to the connected LLM for a spelling/punctuation
//! pass. Only text crosses the network here — audio and video never leave
//! the machine. Segment boundaries are preserved by asking the model to
//! return exactly one corrected line per input line; a line-count mismatch
//! is treated as a failure and the caller falls back to the original
//! transcript rather than risk desyncing timestamps from text.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::settings::LlmSettings;
use crate::timeline::TimedText;

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}
#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

const SYSTEM_PROMPT: &str =
    "You correct spelling and punctuation in Indonesian speech transcripts. \
You will receive one transcript line per input line, numbered. Return exactly the same number of \
lines, in the same order, each corrected for spelling and punctuation only — never merge, split, \
translate, reorder, or add commentary. Reply with only the corrected lines, numbered the same way.";

/// Rewrite every segment's text via the configured provider. Returns new
/// `TimedText` with the same start/end timestamps and only the text swapped.
pub fn rewrite_transcript(
    settings: &LlmSettings,
    access_token: &str,
    segments: &[TimedText],
) -> Result<Vec<TimedText>> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }
    if settings.api_base.is_empty() || settings.model.is_empty() {
        return Err(anyhow!("LLM provider has no api_base/model configured"));
    }

    let numbered = format_numbered(segments);
    let url = format!(
        "{}/chat/completions",
        settings.api_base.trim_end_matches('/')
    );
    let req = ChatRequest {
        model: &settings.model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: SYSTEM_PROMPT.to_string(),
            },
            ChatMessage {
                role: "user",
                content: numbered,
            },
        ],
        temperature: 0.0,
    };

    let resp: ChatResponse = ureq::post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Content-Type", "application/json")
        .send_json(&req)
        .context("calling LLM chat-completions endpoint")?
        .into_json()
        .context("parsing LLM chat-completions response")?;

    let content = resp
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .ok_or_else(|| anyhow!("LLM response had no choices"))?;

    let corrected = parse_numbered(content, segments.len()).ok_or_else(|| {
        anyhow!(
            "LLM returned {} lines, expected {}",
            content.lines().count(),
            segments.len()
        )
    })?;

    Ok(segments
        .iter()
        .zip(corrected)
        .map(|(seg, text)| TimedText::new(seg.start, seg.end, text))
        .collect())
}

fn format_numbered(segments: &[TimedText]) -> String {
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}", i + 1, s.text.replace('\n', " ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse `"1. text\n2. text\n..."` back into `expected` plain strings, in
/// order. `None` if the line count doesn't match — the caller must not use
/// a partial result, since a dropped line would shift every timestamp after
/// it onto the wrong text.
fn parse_numbered(text: &str, expected: usize) -> Option<Vec<String>> {
    let lines: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            line.split_once('.')
                .map(|(_, rest)| rest.trim().to_string())
                .unwrap_or_else(|| line.to_string())
        })
        .collect();
    if lines.len() == expected {
        Some(lines)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TimedText {
        TimedText::new(start, end, text)
    }

    #[test]
    fn numbered_formatting_round_trips() {
        let segs = vec![seg(0.0, 1.0, "halo dunya"), seg(1.0, 2.0, "apa kabar")];
        let formatted = format_numbered(&segs);
        assert_eq!(formatted, "1. halo dunya\n2. apa kabar");
        let parsed = parse_numbered("1. halo dunia\n2. apa kabar?", 2).unwrap();
        assert_eq!(
            parsed,
            vec!["halo dunia".to_string(), "apa kabar?".to_string()]
        );
    }

    #[test]
    fn mismatched_line_count_is_rejected() {
        assert!(parse_numbered("1. only one line", 2).is_none());
    }

    #[test]
    fn blank_lines_are_ignored() {
        let parsed = parse_numbered("1. a\n\n2. b\n", 2).unwrap();
        assert_eq!(parsed, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn empty_segments_short_circuit_without_a_network_call() {
        let settings = LlmSettings::default();
        let out = rewrite_transcript(&settings, "token", &[]).unwrap();
        assert!(out.is_empty());
    }
}
