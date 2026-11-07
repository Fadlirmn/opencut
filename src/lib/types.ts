// Mirrors the Rust types in src-tauri/src. Keep in sync.

export type MediaInfo = {
  path: string;
  duration: number;
  fps: number;
  /// Zero for audio-only media.
  width: number;
  height: number;
  has_video: boolean;
  has_audio: boolean;
  start_timecode: string | null;
};

/// A piece of media that rides along with the reference clip and gets cut at
/// the same points. Video tracks land above the reference in the exported
/// timeline, audio-only tracks below it.
export type LinkedTrack = {
  id: string;
  info: MediaInfo;
  /// Project-time second at which this track's media begins. Project time is
  /// the reference clip's own timeline, so the reference is always at 0.
  offset: number;
  /// True when `offset` was derived by comparing embedded timecodes rather
  /// than assumed to be zero. Surfaced in the UI so a user with no timecode
  /// knows the number is a guess they may need to correct.
  offsetFromTimecode: boolean;
};

/// What `open_track` hands back: the probe plus where the track appears to
/// belong. SMPTE parsing (including drop-frame) lives in Rust; duplicating it
/// in TypeScript would be two implementations to keep in agreement.
export type TrackProbe = {
  info: MediaInfo;
  offset: number;
  offset_from_timecode: boolean;
};

export type CutKind = "keep" | "remove";

export type Cut = {
  start: number;
  end: number;
  kind: CutKind;
  /// Frontend-only flag. A disabled `keep` still appears in the UI but the
  /// store collapses it into a `remove` before sending to the Rust exporter.
  /// Rust never sees this field.
  disabled?: boolean;
};

export type CutList = {
  source_duration: number;
  intervals: Cut[];
};

export type DetectParams = {
  threshold: number;
  min_silence_ms: number;
  min_speech_ms: number;
  pad: number;
  /// Project-time second the analysed audio begins at. Non-zero only when
  /// detection is running on a linked track that starts later than the
  /// reference — the resulting cutlist is always in project time.
  source_offset: number;
};

export type ExportQuality = "source" | "high" | "medium" | "small";
export type ExportResolution = "source" | "1080p" | "720p" | "480p";

export type ExportOptions = {
  quality: ExportQuality;
  resolution: ExportResolution;
};

export type ExportProgressEvent = {
  pct: number;
  message: string;
};

export type SubtitleFormat = "ass" | "srt";

export type DiagnosticInfo = {
  app_version: string;
  target_os: string;
  target_arch: string;
  ffmpeg_path: string | null;
  ffmpeg_exists: boolean;
  ffprobe_path: string | null;
  ffprobe_exists: boolean;
};

// ===== Batch queue (primary UI) =====

export type JobState = "queued" | "running" | "done" | "failed";

export type StageState = "pending" | "running" | "done" | "failed";

/// One step of a pipeline run (probe, extract, detect, transcribe, export…).
export type Stage = {
  key: string;
  label: string;
  state: StageState;
  /// 0-100 within this stage.
  pct: number;
  detail: string;
  elapsed_secs: number;
  eta_secs: number | null;
};

export type Job = {
  id: number;
  name: string;
  path: string;
  state: JobState;
  pct: number;
  message: string;
  output_path: string | null;
  error: string | null;
  stages: Stage[];
};

export type LogEntry = {
  name: string;
  /// Milliseconds since the Unix epoch — format with `new Date(ms)` here,
  /// not in Rust, so locale/timezone stays a frontend concern.
  finished_at_ms: number;
  success: boolean;
  output_path: string | null;
  error: string | null;
  subtitles: boolean;
  /// Detail fields, present on entries written by 0.0.10 and later.
  source_path?: string | null;
  duration_secs?: number | null;
  failed_stage?: string | null;
  stages?: Stage[];
  app_version?: string | null;
  os?: string | null;
};

export type LlmSettings = {
  provider_label: string;
  client_id: string;
  auth_url: string;
  token_url: string;
  scope: string;
  api_base: string;
  model: string;
  auto_rewrite: boolean;
};

export type AppSettings = {
  parallel_enabled: boolean;
  parallel_count: number;
  burn_subtitles: boolean;
  quality: ExportQuality;
  resolution: ExportResolution;
  output_dir: string | null;
  llm: LlmSettings;
  /// Backend migration marker; round-tripped untouched.
  settings_version?: number;
};

export type LlmAuthMode = "api_key" | "oauth" | "none";

export type LlmStatus = {
  connected: boolean;
  provider_label: string;
  auth_mode: LlmAuthMode;
};

/// Whether the whisper model (downloaded on demand, not bundled) is on disk.
export type ModelStatus = {
  present: boolean;
  path: string | null;
  /// A download is already running in the backend.
  downloading: boolean;
  /// Expected download size in bytes.
  size_bytes: number;
};

export type ModelDownloadProgress = {
  downloaded: number;
  total: number;
};
