import {
  getSettings,
  llmCancelConnect as apiLlmCancelConnect,
  llmConnect as apiLlmConnect,
  llmDisconnect as apiLlmDisconnect,
  llmSetApiKey as apiLlmSetApiKey,
  llmStatus as apiLlmStatus,
  llmTest as apiLlmTest,
  logList as apiLogList,
  onQueueUpdate,
  queueAdd,
  queueClearDone,
  queueRemove,
  queueRetry,
  queueSnapshot,
  queueStart as apiQueueStart,
  saveSettings as apiSaveSettings,
} from "./api";
import type { AppSettings, Job, LlmStatus, LogEntry } from "./types";

export type Page = "upload" | "log" | "settings" | "profile";

const DEFAULT_LLM_STATUS: LlmStatus = { connected: false, provider_label: "", auth_mode: "none" };

class BatchStore {
  page = $state<Page>("upload");

  /// The single source of truth for job state — always refreshed from the
  /// `queue-updated` event, never mutated locally, so a cancelled-but-
  /// still-running job never looks removed before the worker actually stops.
  jobs = $state<Job[]>([]);

  settings = $state<AppSettings | null>(null);
  settingsError = $state<string | null>(null);
  savingSettings = $state(false);

  logs = $state<LogEntry[]>([]);
  logsError = $state<string | null>(null);

  llm = $state<LlmStatus>(DEFAULT_LLM_STATUS);
  llmBusy = $state(false);
  llmError = $state<string | null>(null);
  llmTestResult = $state<string | null>(null);

  private unlistenQueue: (() => void) | null = null;
  private initialized = false;

  /// Called once from the app shell. Loads the initial snapshot of
  /// everything the sidebar/pages need and subscribes to queue updates;
  /// safe to call more than once (e.g. hot reload during development).
  async init() {
    if (this.initialized) return;
    this.initialized = true;
    await Promise.all([this.refreshSettings(), this.refreshLlmStatus(), this.refreshLogs()]);
    try {
      this.jobs = await queueSnapshot();
    } catch {
      /* fresh app data dir — nothing queued yet */
    }
    this.unlistenQueue = await onQueueUpdate((jobs) => {
      this.jobs = jobs;
    });
  }

  async addFiles(paths: string[]) {
    const real = paths.filter((p) => p.trim().length > 0);
    if (real.length === 0) return;
    // queue_add also emits queue-updated, so this.jobs updates via the
    // listener; the direct return value isn't needed here.
    await queueAdd(real);
  }

  async start() {
    await apiQueueStart();
  }

  async remove(id: number) {
    await queueRemove(id);
  }

  async retry(id: number) {
    await queueRetry(id);
  }

  async clearDone() {
    await queueClearDone();
  }

  async refreshSettings() {
    try {
      this.settings = await getSettings();
      this.settingsError = null;
    } catch (e) {
      this.settingsError = String(e);
    }
  }

  async saveSettings(next: AppSettings) {
    this.savingSettings = true;
    this.settingsError = null;
    try {
      await apiSaveSettings(next);
      this.settings = next;
    } catch (e) {
      this.settingsError = String(e);
    } finally {
      this.savingSettings = false;
    }
  }

  async refreshLogs() {
    try {
      this.logs = await apiLogList();
      this.logsError = null;
    } catch (e) {
      this.logsError = String(e);
    }
  }

  async refreshLlmStatus() {
    try {
      this.llm = await apiLlmStatus();
    } catch {
      // Keyring access failing (locked, unavailable headless session, …)
      // reads as "not connected" rather than surfacing as an app error —
      // the Settings page's own connect flow is where a real failure shows.
      this.llm = DEFAULT_LLM_STATUS;
    }
  }

  async connectLlm() {
    this.llmBusy = true;
    this.llmError = null;
    try {
      await apiLlmConnect();
      await this.refreshLlmStatus();
    } catch (e) {
      this.llmError = String(e);
    } finally {
      this.llmBusy = false;
    }
  }

  async cancelLlmConnect() {
    await apiLlmCancelConnect();
  }

  async setLlmApiKey(key: string) {
    this.llmBusy = true;
    this.llmError = null;
    try {
      await apiLlmSetApiKey(key);
      await this.refreshLlmStatus();
    } catch (e) {
      this.llmError = String(e);
    } finally {
      this.llmBusy = false;
    }
  }

  async disconnectLlm() {
    await apiLlmDisconnect();
    await this.refreshLlmStatus();
  }

  async testLlm() {
    this.llmTestResult = null;
    try {
      await apiLlmTest();
      this.llmTestResult = "berhasil";
    } catch (e) {
      this.llmTestResult = String(e);
    }
  }

  dispose() {
    this.unlistenQueue?.();
    this.unlistenQueue = null;
  }
}

export const batch = new BatchStore();
