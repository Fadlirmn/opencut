//! Dev-only: time the transcription stage on one video, the way the queue
//! pipeline runs it, so speed changes can be measured without a full app
//! build.
//!
//! Usage:
//!   cargo run --release --example bench_transcribe -- <video> [--model PATH] [--full]
//!
//! By default transcribes only the kept (speech) parts, as the app does.
//! `--full` transcribes the whole track instead, for comparison.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use autocut_lib::audio::{extract_pcm, VAD_SAMPLE_RATE};
use autocut_lib::cutlist::CutList;
use autocut_lib::probe::probe;
use autocut_lib::timeline::{map_to_output, SpeechTrack};
use autocut_lib::transcribe::transcribe;
use autocut_lib::vad::{score_chunks, segments_from_scores, VadParams};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut video: Option<PathBuf> = None;
    let mut model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("models")
        .join("ggml-large-v3-turbo-q5_0.bin");
    let mut full = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model" => model = PathBuf::from(args.next().expect("--model PATH")),
            "--full" => full = true,
            _ => video = Some(PathBuf::from(a)),
        }
    }
    let video = video.expect("usage: bench_transcribe <video> [--model PATH] [--full]");
    let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    let triple = "x86_64-unknown-linux-gnu";
    let ffmpeg = bin.join(format!("ffmpeg-{triple}"));
    let ffprobe = bin.join(format!("ffprobe-{triple}"));

    let info = probe(&ffprobe, &video)?;
    let samples = extract_pcm(&ffmpeg, &video)?;
    let scores = score_chunks(&samples, None)?;
    let segs = segments_from_scores(&scores, VadParams::default(), 0.0);
    let cutlist = CutList::from_speech_segments(&segs, info.duration, 0.3);

    let cancel = Arc::new(AtomicBool::new(false));
    let t = Instant::now();
    let source_segments = if full {
        transcribe(&model, &samples, cancel, |_| {})?
    } else {
        let speech = SpeechTrack::build(&samples, VAD_SAMPLE_RATE, &cutlist);
        eprintln!(
            "speech track {:.1}s of {:.1}s source",
            speech.duration(),
            info.duration
        );
        speech.segments_to_source(&transcribe(&model, &speech.samples, cancel, |_| {})?)
    };
    let secs = t.elapsed().as_secs_f64();

    println!(
        "mode={} transcribe={secs:.1}s source={:.1}s",
        if full { "full" } else { "speech" },
        info.duration
    );
    for s in map_to_output(&cutlist, &source_segments) {
        println!("  [{:6.2} - {:6.2}] {}", s.start, s.end, s.text);
    }
    Ok(())
}
