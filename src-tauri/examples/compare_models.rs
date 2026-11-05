//! Dev-only tool to A/B test Whisper models (and optionally ffmpeg's
//! `afftdn` denoise filter) on a video/audio file, side by side.
//!
//! Not wired into the app: for manually judging transcription quality per
//! the project's model/denoise evaluation checklist before changing the
//! bundled model.
//!
//! Usage:
//!   cargo run --example compare_models -- <video> [--model PATH ...] [--denoise]
//!
//! With no `--model`, compares against only the bundled model. Pass
//! `--model` once per extra model file (e.g. a manually downloaded
//! ggml-large-v3.bin) to compare it against the bundled one in the same run.
//! `--denoise` additionally extracts a second, denoised PCM track and runs
//! every model against it too, so cleaned vs. raw audio can be compared.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use autocut_lib::{audio, binaries, transcribe};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut video: Option<PathBuf> = None;
    let mut models: Vec<PathBuf> = Vec::new();
    let mut denoise = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => {
                let path = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--model needs a path"))?;
                models.push(PathBuf::from(path));
            }
            "--denoise" => denoise = true,
            other if video.is_none() => video = Some(PathBuf::from(other)),
            other => anyhow::bail!("unexpected argument: {other}"),
        }
    }
    let video = video.ok_or_else(|| {
        anyhow::anyhow!("usage: compare_models <video> [--model PATH ...] [--denoise]")
    })?;

    if models.is_empty() {
        models.push(binaries::model_path(None)?);
    }

    let ffmpeg = binaries::ffmpeg_path(None)?;

    println!("extracting audio from {}", video.display());
    let raw = audio::extract_pcm(&ffmpeg, &video)?;
    run_models(&models, &raw, "raw")?;

    if denoise {
        println!(
            "\nextracting denoised audio (afftdn) from {}",
            video.display()
        );
        let denoised = audio::extract_pcm_denoised(&ffmpeg, &video, None)?;
        run_models(&models, &denoised, "denoised")?;
    }

    Ok(())
}

fn run_models(models: &[PathBuf], samples: &[i16], label: &str) -> anyhow::Result<()> {
    for model in models {
        println!("\n--- model={} audio={label} ---", model.display());
        let start = Instant::now();
        let segments = transcribe::transcribe(model, samples, Arc::new(AtomicBool::new(false)))?;
        let elapsed = start.elapsed();
        for seg in &segments {
            println!("[{:>7.2}-{:>7.2}] {}", seg.start, seg.end, seg.text);
        }
        println!(
            "({} segments, {:.1}s audio, {:.1}s wall clock)",
            segments.len(),
            samples.len() as f64 / audio::VAD_SAMPLE_RATE as f64,
            elapsed.as_secs_f64()
        );
    }
    Ok(())
}
