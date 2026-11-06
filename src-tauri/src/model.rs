//! On-demand download of the Whisper model.
//!
//! The model is ~550 MB, more than everything else in the installer combined,
//! so it is no longer bundled. The first time the user asks for subtitles the
//! frontend offers to fetch it into `<app_data_dir>/models/`, which is where
//! [`crate::binaries::model_path`] looks first.
//!
//! The download streams into `<name>.part` while hashing, verifies the pinned
//! sha256, and only then renames onto the final name. A crash, a cancel, or a
//! truncated/corrupt transfer therefore never leaves something at the real
//! path that whisper would try (and fail) to load.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

pub const MODEL_FILENAME: &str = "ggml-large-v3-turbo-q5_0.bin";
pub const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin";
/// git-lfs content hash of the file on the ggerganov/whisper.cpp HF repo
/// (also served as its `x-linked-etag`). Changes only if that upload is
/// ever replaced, in which case the download fails loudly instead of
/// silently loading a different model.
pub const MODEL_SHA256: &str = "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2";
/// Expected size, used for the progress bar only when the server omits
/// Content-Length. The sha256 is what actually guards integrity.
pub const MODEL_SIZE: u64 = 574_041_195;

/// Error text for a user-initiated cancel. The frontend matches on
/// "dibatalkan" to tell a cancel apart from a real failure.
pub const CANCELLED_MSG: &str = "unduhan model dibatalkan";

/// Emit a progress callback at most once per this many bytes.
const PROGRESS_STEP: u64 = 1024 * 1024;

pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("models")
}

/// Where a downloaded model lives (and where [`download`] writes to).
pub fn downloaded_model_path(app_data_dir: &Path) -> PathBuf {
    models_dir(app_data_dir).join(MODEL_FILENAME)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// A model file counts as present only if it is a non-empty regular file.
/// Zero-length files are stubs (old `AUTOCUT_STUB_MODEL` builds) or the
/// remains of a failed copy, and whisper would choke on them.
pub fn is_usable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// Fetch the pinned model from Hugging Face into `dest`. `on_progress` gets
/// `(downloaded_bytes, total_bytes)`. Checked against `cancel` between
/// chunks; a stalled connection is bounded by the read timeout.
pub fn download(dest: &Path, cancel: &AtomicBool, on_progress: impl FnMut(u64, u64)) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        bail!(CANCELLED_MSG);
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent
        .get(MODEL_URL)
        .call()
        .map_err(|e| anyhow!("gagal menghubungi huggingface.co: {e}"))?;
    let total = resp
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(MODEL_SIZE);
    write_verified(
        resp.into_reader(),
        total,
        dest,
        MODEL_SHA256,
        cancel,
        on_progress,
    )
}

/// Stream `reader` into `dest`'s `.part` sibling while hashing, check the
/// hash, then rename into place. On any failure the `.part` file is removed
/// and `dest` is left untouched.
pub fn write_verified<R: Read>(
    mut reader: R,
    total: u64,
    dest: &Path,
    expected_sha256: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).with_context(|| format!("membuat folder {}", dir.display()))?;
    }
    let part = part_path(dest);

    let result = (|| -> Result<()> {
        let file =
            File::create(&part).with_context(|| format!("membuat file {}", part.display()))?;
        let mut out = BufWriter::new(file);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut downloaded: u64 = 0;
        let mut last_reported: u64 = 0;
        on_progress(0, total);

        loop {
            if cancel.load(Ordering::SeqCst) {
                bail!(CANCELLED_MSG);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("koneksi unduhan terputus"),
            };
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])
                .context("menulis model ke disk (ruang disk penuh?)")?;
            downloaded += n as u64;
            if downloaded - last_reported >= PROGRESS_STEP {
                last_reported = downloaded;
                on_progress(downloaded, total.max(downloaded));
            }
        }
        on_progress(downloaded, total.max(downloaded));

        let file = out
            .into_inner()
            .map_err(|e| e.into_error())
            .context("menulis model ke disk")?;
        file.sync_all().context("menulis model ke disk")?;
        drop(file);

        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            bail!(
                "checksum model tidak cocok (unduhan mungkin terputus atau rusak). \
                 diharapkan {expected_sha256}, didapat {actual}"
            );
        }
        Ok(())
    })();

    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    // std::fs::rename replaces an existing destination on every platform we
    // ship (MoveFileExW with REPLACE_EXISTING on Windows).
    fs::rename(&part, dest).with_context(|| {
        let _ = fs::remove_file(&part);
        format!("memindahkan model ke {}", dest.display())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "autocut-model-test-{tag}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha_hex(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn writes_and_renames_when_hash_matches() {
        let dir = tmp_dir("ok");
        let dest = dir.join("models").join(MODEL_FILENAME);
        let data = vec![7u8; 3 * 1024 * 1024 + 17];
        let mut calls = Vec::new();
        write_verified(
            Cursor::new(data.clone()),
            data.len() as u64,
            &dest,
            &sha_hex(&data),
            &AtomicBool::new(false),
            |d, t| calls.push((d, t)),
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), data);
        assert!(!part_path(&dest).exists());
        assert_eq!(calls.first(), Some(&(0, data.len() as u64)));
        assert_eq!(calls.last(), Some(&(data.len() as u64, data.len() as u64)));
        assert!(is_usable(&dest));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hash_mismatch_leaves_nothing_behind() {
        let dir = tmp_dir("bad");
        let dest = dir.join(MODEL_FILENAME);
        let err = write_verified(
            Cursor::new(b"not a model".to_vec()),
            11,
            &dest,
            MODEL_SHA256,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("checksum"));
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_aborts_and_cleans_up() {
        let dir = tmp_dir("cancel");
        let dest = dir.join(MODEL_FILENAME);
        let err = write_verified(
            Cursor::new(vec![0u8; 1024]),
            1024,
            &dest,
            MODEL_SHA256,
            &AtomicBool::new(true),
            |_, _| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("dibatalkan"));
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_file_is_not_usable() {
        let dir = tmp_dir("empty");
        let p = dir.join(MODEL_FILENAME);
        fs::write(&p, b"").unwrap();
        assert!(!is_usable(&p));
        assert!(!is_usable(&dir.join("missing.bin")));
        fs::write(&p, b"x").unwrap();
        assert!(is_usable(&p));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn part_path_appends_suffix() {
        let p = Path::new("/a/models").join(MODEL_FILENAME);
        assert_eq!(
            part_path(&p),
            Path::new("/a/models").join(format!("{MODEL_FILENAME}.part"))
        );
    }
}
