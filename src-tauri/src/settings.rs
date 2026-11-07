//! Persisted app settings: batch-processing defaults, parallel worker count,
//! and the LLM connection config used to rewrite transcripts. Stored as JSON
//! in the Tauri app-data directory (`settings.json`); never touched by the
//! build, unlike the model/sidecar paths in `binaries.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::commands::{ExportQuality, ExportResolution};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmSettings {
    /// Human label only (e.g. "Anthropic (Claude)"); carries no behavior.
    pub provider_label: String,
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
    /// OpenAI-compatible chat-completions base, e.g. "https://api.openai.com/v1".
    pub api_base: String,
    pub model: String,
    /// Run the rewrite automatically after every transcription, once connected.
    pub auto_rewrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Run more than one queued video through the pipeline at once.
    #[serde(default)]
    pub parallel_enabled: bool,
    /// Clamped to [1,4] on load; meaningless (and ignored by the queue) when
    /// `parallel_enabled` is false.
    #[serde(default = "default_parallel_count")]
    pub parallel_count: u32,
    #[serde(default)]
    pub burn_subtitles: bool,
    #[serde(default = "default_quality")]
    pub quality: ExportQuality,
    #[serde(default = "default_resolution")]
    pub resolution: ExportResolution,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub llm: LlmSettings,
    /// Bumped when a stored default must be migrated on load. Absent from
    /// files written before v1; the frontend round-trips it unchanged, and a
    /// save that omits it is treated as already current.
    #[serde(default = "current_settings_version")]
    pub settings_version: u32,
}

/// v1: the "source" (asli) export quality became the default. Files without
/// a version predate it and still carry the old "medium" default, so they
/// get switched to source once.
const SETTINGS_VERSION: u32 = 1;

fn current_settings_version() -> u32 {
    SETTINGS_VERSION
}

fn default_parallel_count() -> u32 {
    2
}
fn default_quality() -> ExportQuality {
    ExportQuality::Source
}
fn default_resolution() -> ExportResolution {
    ExportResolution::Source
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            parallel_enabled: false,
            parallel_count: default_parallel_count(),
            burn_subtitles: true,
            quality: default_quality(),
            resolution: default_resolution(),
            output_dir: None,
            llm: LlmSettings::default(),
            settings_version: SETTINGS_VERSION,
        }
    }
}

impl AppSettings {
    /// Worker count the queue should actually run with right now.
    pub fn effective_concurrency(&self) -> usize {
        if self.parallel_enabled {
            self.parallel_count.clamp(1, 4) as usize
        } else {
            1
        }
    }
}

fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

pub fn load(app_data_dir: &Path) -> AppSettings {
    let path = settings_path(app_data_dir);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| parse(&s))
        .unwrap_or_default()
}

/// Parse stored settings, migrating files written by older versions.
fn parse(raw: &str) -> Option<AppSettings> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let stored_version = value
        .get("settings_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let mut settings: AppSettings = serde_json::from_value(value).ok()?;
    if stored_version < 1 {
        settings.quality = ExportQuality::Source;
    }
    settings.settings_version = SETTINGS_VERSION;
    Some(settings)
}

pub fn save(app_data_dir: &Path, settings: &AppSettings) -> Result<()> {
    fs::create_dir_all(app_data_dir)
        .with_context(|| format!("creating app data dir at {}", app_data_dir.display()))?;
    let path = settings_path(app_data_dir);
    let json = serde_json::to_string_pretty(settings).context("serializing settings")?;
    fs::write(&path, json).with_context(|| format!("writing settings to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_concurrency_is_one_when_parallel_disabled() {
        let s = AppSettings {
            parallel_enabled: false,
            parallel_count: 4,
            ..Default::default()
        };
        assert_eq!(s.effective_concurrency(), 1);
    }

    #[test]
    fn effective_concurrency_clamps_to_four() {
        let s = AppSettings {
            parallel_enabled: true,
            parallel_count: 99,
            ..Default::default()
        };
        assert_eq!(s.effective_concurrency(), 4);
    }

    #[test]
    fn effective_concurrency_clamps_to_at_least_one() {
        let s = AppSettings {
            parallel_enabled: true,
            parallel_count: 0,
            ..Default::default()
        };
        assert_eq!(s.effective_concurrency(), 1);
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let dir = std::env::temp_dir().join("autocut-settings-test-missing");
        let _ = fs::remove_dir_all(&dir);
        let loaded = load(&dir);
        assert!(!loaded.parallel_enabled);
    }

    #[test]
    fn pre_v1_settings_switch_to_source_quality() {
        let s = parse(r#"{"quality": "medium", "resolution": "720p"}"#).unwrap();
        assert!(matches!(s.quality, ExportQuality::Source));
        assert!(matches!(s.resolution, ExportResolution::P720));
        assert_eq!(s.settings_version, SETTINGS_VERSION);
    }

    #[test]
    fn current_settings_keep_the_chosen_quality() {
        let s = parse(r#"{"quality": "small", "settings_version": 1}"#).unwrap();
        assert!(matches!(s.quality, ExportQuality::Small));
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir =
            std::env::temp_dir().join(format!("autocut-settings-test-{}", std::process::id()));
        let s = AppSettings {
            parallel_enabled: true,
            parallel_count: 3,
            llm: LlmSettings {
                provider_label: "Test Provider".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        save(&dir, &s).unwrap();
        let loaded = load(&dir);
        assert!(loaded.parallel_enabled);
        assert_eq!(loaded.parallel_count, 3);
        assert_eq!(loaded.llm.provider_label, "Test Provider");
        let _ = fs::remove_dir_all(&dir);
    }
}
