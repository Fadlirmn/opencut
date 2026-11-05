import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  CutList,
  DetectParams,
  DiagnosticInfo,
  ExportOptions,
  ExportProgressEvent,
  Job,
  LinkedTrack,
  LlmStatus,
  LogEntry,
  MediaInfo,
  SubtitleFormat,
  TrackProbe,
} from "./types";

export async function openVideo(path: string): Promise<MediaInfo> {
  return invoke<MediaInfo>("open_video", { path });
}

/// Probe a file to ride along as a linked track, and ask where it belongs.
/// The reference's timecode and rate go along so the offset can be derived
/// from embedded timecode when both sides have one.
export async function openTrack(
  path: string,
  referenceTimecode: string | null,
  fps: number,
): Promise<TrackProbe> {
  return invoke<TrackProbe>("open_track", {
    path,
    referenceTimecode,
    fps,
  });
}

export async function diagnosticInfo(): Promise<DiagnosticInfo> {
  return invoke<DiagnosticInfo>("diagnostic_info");
}

export async function computeWaveform(
  path: string,
  targetBins: number,
): Promise<number[]> {
  return invoke<number[]>("compute_waveform", { path, targetBins });
}

export async function detectSilence(
  path: string,
  duration: number,
  params: DetectParams,
): Promise<{ cutlist: CutList }> {
  return invoke<{ cutlist: CutList }>("detect_silence", {
    path,
    duration,
    params,
  });
}

export async function cancelDetect(): Promise<void> {
  await invoke("cancel_detect");
}

export async function exportMp4(
  source: string,
  output: string,
  cutlist: CutList,
  options: ExportOptions,
  hasAudio: boolean,
  subtitleAss: string | null = null,
): Promise<void> {
  await invoke("export_mp4", {
    args: {
      source,
      output,
      cutlist,
      quality: options.quality,
      resolution: options.resolution,
      has_audio: hasAudio,
      subtitle_ass: subtitleAss,
    },
  });
}

export async function cancelExport(): Promise<void> {
  await invoke("cancel_export");
}

/// Transcribe `path`'s audio (reusing the already-decoded/cached PCM) and
/// render it as subtitles already re-based onto the cut timeline.
export async function generateSubtitles(
  path: string,
  cutlist: CutList,
  video: { width: number; height: number },
  format: SubtitleFormat,
): Promise<string> {
  return invoke<string>("generate_subtitles", { path, cutlist, video, format });
}

export async function cancelSubtitles(): Promise<void> {
  await invoke("cancel_subtitles");
}

export async function saveSubtitles(path: string, contents: string): Promise<void> {
  await invoke("save_subtitles", { path, contents });
}

export async function cancelWaveform(): Promise<void> {
  await invoke("cancel_waveform");
}

export async function revealInFinder(path: string): Promise<void> {
  await invoke("reveal_in_finder", { path });
}

export async function openWithDefaultApp(path: string): Promise<void> {
  await invoke("open_with_default_app", { path });
}

export async function exportFcpxml(
  source: string,
  output: string,
  cutlist: CutList,
  fps: number,
  startTimecode: string | null,
  title: string,
  hasAudio: boolean,
  tracks: LinkedTrack[],
): Promise<void> {
  await invoke("export_fcpxml", {
    args: {
      source,
      output,
      cutlist,
      fps,
      start_timecode: startTimecode,
      title,
      has_audio: hasAudio,
      tracks: tracks.map((t) => ({
        source: t.info.path,
        name: t.info.path.split(/[/\\]/).pop() ?? t.info.path,
        duration: t.info.duration,
        start_timecode: t.info.start_timecode,
        has_video: t.info.has_video,
        has_audio: t.info.has_audio,
        offset: t.offset,
      })),
    },
  });
}

export function onExportProgress(
  handler: (e: ExportProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ExportProgressEvent>("export-progress", (event) => {
    handler(event.payload);
  });
}

// ===================== Batch queue (primary UI) =====================

export async function queueAdd(paths: string[]): Promise<Job[]> {
  return invoke<Job[]>("queue_add", { paths });
}

export async function queueSnapshot(): Promise<Job[]> {
  return invoke<Job[]>("queue_snapshot");
}

export async function queueRemove(id: number): Promise<void> {
  await invoke("queue_remove", { id });
}

export async function queueRetry(id: number): Promise<void> {
  await invoke("queue_retry", { id });
}

export async function queueClearDone(): Promise<void> {
  await invoke("queue_clear_done");
}

export async function queueStart(): Promise<void> {
  await invoke("queue_start");
}

/// The whole queue snapshot, re-sent on every state change (a job's
/// progress tick, a finish, an add/remove) — simpler for a dynamic list
/// than diffing individual job events on the frontend.
export function onQueueUpdate(handler: (jobs: Job[]) => void): Promise<UnlistenFn> {
  return listen<Job[]>("queue-updated", (event) => {
    handler(event.payload);
  });
}

// ===================== Settings =====================

export async function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

export async function saveSettings(settings: AppSettings): Promise<void> {
  await invoke("save_settings", { settings });
}

// ===================== Log =====================

export async function logList(): Promise<LogEntry[]> {
  return invoke<LogEntry[]>("log_list");
}

// ===================== LLM connection (OAuth) =====================

export async function llmStatus(): Promise<LlmStatus> {
  return invoke<LlmStatus>("llm_status");
}

export async function llmConnect(): Promise<void> {
  await invoke("llm_connect");
}

export async function llmCancelConnect(): Promise<void> {
  await invoke("llm_cancel_connect");
}

export async function llmDisconnect(): Promise<void> {
  await invoke("llm_disconnect");
}

export async function llmTest(): Promise<void> {
  await invoke("llm_test");
}
