//! Resolve ffmpeg/ffprobe sidecar paths at runtime.
//!
//! Tauri renames sidecars during bundling so the production layout is just
//! `<resource_dir>/ffmpeg[.exe]` (no triple suffix). In dev mode the cargo
//! manifest dir wins (binaries/ffmpeg-<triple>).
//!
//! `binary_path` tries dev paths first (cheap stat), then resource-dir paths.
//! If neither exists the user almost certainly hit a packaging bug; we return
//! Err so the calling command surfaces a clear error.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};

#[derive(Clone, Copy)]
enum Tool {
    Ffmpeg,
    Ffprobe,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
        }
    }
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
compile_error!("autocut ffmpeg sidecars are not configured for this target");

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TARGET_TRIPLE: &str = "aarch64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const TARGET_TRIPLE: &str = "x86_64-apple-darwin";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TARGET_TRIPLE: &str = "x86_64-unknown-linux-gnu";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const TARGET_TRIPLE: &str = "aarch64-unknown-linux-gnu";
#[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
const TARGET_TRIPLE: &str = "x86_64-pc-windows-msvc";
#[cfg(all(
    target_os = "windows",
    target_arch = "x86_64",
    not(target_env = "msvc")
))]
const TARGET_TRIPLE: &str = "x86_64-pc-windows-gnu";

static FFMPEG_PATH: OnceLock<PathBuf> = OnceLock::new();
static FFPROBE_PATH: OnceLock<PathBuf> = OnceLock::new();
use crate::model::MODEL_FILENAME as WHISPER_MODEL_FILENAME;

fn ext() -> &'static str {
    if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    }
}

fn binary_path(tool: Tool, resource_dir: Option<&Path>) -> Result<PathBuf> {
    let leaf = tool.name();
    let suffix = ext();
    let triple = TARGET_TRIPLE;

    // Candidates, tried in priority order. The most important one in
    // production is the directory holding the running executable: on macOS
    // Tauri places sidecar binaries next to the main app binary in
    // Contents/MacOS, NOT in Contents/Resources, so app.path().resource_dir()
    // alone misses them. Debug builds also fall back to the build-time dev
    // path (manifest_dir/binaries) so cargo-run / cargo-test work without
    // bundling; release builds skip it because that path is the CI runner's
    // checkout and only confuses the error message. Finally a tool on PATH
    // is accepted, so a bare exe still works where ffmpeg is installed.
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(format!("{leaf}{suffix}")));
            candidates.push(parent.join(format!("{leaf}-{triple}{suffix}")));
        }
    }
    if let Some(rd) = resource_dir {
        candidates.push(rd.join(format!("{leaf}{suffix}")));
        candidates.push(rd.join(format!("{leaf}-{triple}{suffix}")));
    }
    if cfg!(debug_assertions) {
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join(format!("{leaf}-{triple}{suffix}")),
        );
    }

    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let c = dir.join(format!("{leaf}{suffix}"));
            if c.is_file() {
                return Ok(c);
            }
        }
    }

    let attempted = candidates
        .iter()
        .map(|p| p.display().to_string())
        .chain(std::iter::once("PATH".to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow!(
        "could not locate {leaf} binary. tried: {attempted}"
    ))
}

fn cached_binary_path(
    cache: &OnceLock<PathBuf>,
    tool: Tool,
    resource_dir: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = cache.get() {
        return Ok(path.clone());
    }
    let path = binary_path(tool, resource_dir)?;
    let _ = cache.set(path.clone());
    Ok(path)
}

pub fn ffmpeg_path(resource_dir: Option<&Path>) -> Result<PathBuf> {
    cached_binary_path(&FFMPEG_PATH, Tool::Ffmpeg, resource_dir).context("resolving ffmpeg")
}

pub fn ffprobe_path(resource_dir: Option<&Path>) -> Result<PathBuf> {
    cached_binary_path(&FFPROBE_PATH, Tool::Ffprobe, resource_dir).context("resolving ffprobe")
}

/// Resolve the Whisper model. It is no longer bundled (it would more than
/// double the installer); the app downloads it on demand into
/// `<app_data_dir>/models/` (see [`crate::model`]), so that location wins.
/// The old bundled locations (next to the executable, under the resource
/// dir, and the dev-build `src-tauri/models/`) stay as cheap fallbacks so an
/// older install or a developer checkout that already has the file keeps
/// working without a re-download.
///
/// Deliberately not cached: the answer changes the moment a download
/// finishes, and a few stats per transcription cost nothing. Zero-length
/// files are skipped (stubs from `AUTOCUT_STUB_MODEL` builds), since whisper
/// cannot load them.
pub fn model_path(app_data_dir: &Path, resource_dir: Option<&Path>) -> Result<PathBuf> {
    let candidates = model_candidates(app_data_dir, resource_dir);
    if let Some(found) = candidates.iter().find(|c| crate::model::is_usable(c)) {
        return Ok(found.clone());
    }
    let attempted = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow!(
        "model whisper belum diunduh (dicari di: {attempted})"
    ))
}

fn model_candidates(app_data_dir: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![crate::model::downloaded_model_path(app_data_dir)];

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(WHISPER_MODEL_FILENAME));
        }
    }
    if let Some(rd) = resource_dir {
        // Where installers up to 0.0.9 placed the bundled model: Tauri keeps
        // a `resources` entry's relative `models/` subpath under
        // resource_dir. The flat candidate is a harmless extra fallback.
        candidates.push(rd.join("models").join(WHISPER_MODEL_FILENAME));
        candidates.push(rd.join(WHISPER_MODEL_FILENAME));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("models")
            .join(WHISPER_MODEL_FILENAME),
    );
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloaded_model_is_preferred() {
        let dir = std::env::temp_dir().join(format!(
            "autocut-binaries-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let downloaded = crate::model::downloaded_model_path(&dir);
        assert_eq!(model_candidates(&dir, None)[0], downloaded);
        std::fs::create_dir_all(downloaded.parent().unwrap()).unwrap();
        std::fs::write(&downloaded, b"model").unwrap();
        assert_eq!(model_path(&dir, None).unwrap(), downloaded);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
