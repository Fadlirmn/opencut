//! Append-only local record of finished batch jobs, read by the Log page.
//! Plain JSON file in the app-data dir — this is a desktop log, not a
//! database; a few hundred entries is the realistic ceiling.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub name: String,
    /// Milliseconds since the Unix epoch; the frontend formats it with
    /// `Intl`/`Date`, so no date-formatting dependency belongs here.
    pub finished_at_ms: u64,
    pub success: bool,
    pub output_path: Option<String>,
    pub error: Option<String>,
    pub subtitles: bool,
}

const MAX_ENTRIES: usize = 500;

fn log_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("logs.json")
}

pub fn append(app_data_dir: &Path, entry: LogEntry) -> Result<()> {
    fs::create_dir_all(app_data_dir)
        .with_context(|| format!("creating app data dir at {}", app_data_dir.display()))?;
    let mut entries = list(app_data_dir).unwrap_or_default();
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);
    let path = log_path(app_data_dir);
    let json = serde_json::to_string_pretty(&entries).context("serializing log entries")?;
    fs::write(&path, json).with_context(|| format!("writing log to {}", path.display()))
}

pub fn list(app_data_dir: &Path) -> Result<Vec<LogEntry>> {
    let path = log_path(app_data_dir);
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).context("parsing logs.json"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).context(format!("reading {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "autocut-logstore-test-{label}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let dir = tmp_dir("missing");
        let _ = fs::remove_dir_all(&dir);
        assert!(list(&dir).unwrap().is_empty());
    }

    #[test]
    fn append_inserts_newest_first() {
        let dir = tmp_dir("order");
        let _ = fs::remove_dir_all(&dir);
        append(
            &dir,
            LogEntry {
                name: "a.mp4".into(),
                finished_at_ms: 1,
                success: true,
                output_path: None,
                error: None,
                subtitles: false,
            },
        )
        .unwrap();
        append(
            &dir,
            LogEntry {
                name: "b.mp4".into(),
                finished_at_ms: 2,
                success: true,
                output_path: None,
                error: None,
                subtitles: false,
            },
        )
        .unwrap();
        let entries = list(&dir).unwrap();
        assert_eq!(entries[0].name, "b.mp4");
        assert_eq!(entries[1].name, "a.mp4");
        let _ = fs::remove_dir_all(&dir);
    }
}
