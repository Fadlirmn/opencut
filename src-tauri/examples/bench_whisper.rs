//! Dev-only: time whisper alone on a raw PCM file, so the same audio can be
//! benchmarked across platforms/builds without ffmpeg or the VAD.
//!
//! Usage:
//!   bench_whisper <audio.s16le> <model.bin> [threads]
//!
//! `audio.s16le` is 16kHz mono signed 16-bit little-endian PCM, e.g.:
//!   ffmpeg -i in.mp4 -vn -ac 1 -ar 16000 -f s16le audio.s16le
//!
//! Prints whisper's own system info (which SIMD paths the build has), the
//! thread count, the load and transcription times, and the text.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use autocut_lib::transcribe::transcribe;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: bench_whisper <audio.s16le> <model.bin> [threads]";
    let pcm_path = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!(usage))?);
    let model = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!(usage))?);
    if let Some(t) = args.next() {
        std::env::set_var("AUTOCUT_WHISPER_THREADS", t);
    }

    let bytes = std::fs::read(&pcm_path)?;
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();

    println!("system_info: {}", whisper_rs::print_system_info());
    println!(
        "logical_cpus: {}",
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    println!(
        "threads: {}",
        std::env::var("AUTOCUT_WHISPER_THREADS").unwrap_or_else(|_| "auto".into())
    );
    println!("audio: {:.1}s", samples.len() as f64 / 16_000.0);

    let t = Instant::now();
    let segments = transcribe(&model, &samples, Arc::new(AtomicBool::new(false)), |_| {})?;
    println!("transcribe_total: {:.1}s", t.elapsed().as_secs_f64());
    for s in segments {
        println!("  [{:6.2} - {:6.2}] {}", s.start, s.end, s.text);
    }
    Ok(())
}
