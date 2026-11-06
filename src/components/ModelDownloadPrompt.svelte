<script lang="ts">
  // Shown wherever subtitles can be requested while the whisper model is
  // missing. The model (~550 MB) is no longer bundled with the installer, so
  // the first subtitle run has to fetch it once from Hugging Face.
  import { formatBytes, whisperModel } from "../lib/store.svelte";

  let {
    onready,
    ondismiss,
  }: {
    /// Called once the model is downloaded and verified.
    onready?: () => void;
    /// Optional "nanti saja" action, e.g. to turn subtitles back off.
    ondismiss?: () => void;
  } = $props();

  let pct = $derived(
    whisperModel.progress && whisperModel.progress.total > 0
      ? Math.min(100, (whisperModel.progress.downloaded / whisperModel.progress.total) * 100)
      : 0,
  );

  async function start() {
    if (await whisperModel.download()) onready?.();
  }
</script>

<div class="model-prompt">
  {#if whisperModel.downloading}
    <div class="mp-title">Mengunduh model subtitle…</div>
    <div class="mp-progress">
      <div class="mp-bar"><div class="mp-fill" style:width="{pct}%"></div></div>
      <span class="mp-pct mono">{pct.toFixed(0)}%</span>
    </div>
    <div class="mp-row">
      <span class="mp-sub mono muted-2">
        {whisperModel.progress
          ? `${formatBytes(whisperModel.progress.downloaded)} / ${formatBytes(whisperModel.progress.total)}`
          : "menghubungkan…"}
      </span>
      <button class="btn btn-ghost btn-sm" type="button" onclick={() => whisperModel.cancel()}>
        Batalkan
      </button>
    </div>
  {:else}
    <div class="mp-title">Model subtitle belum diunduh</div>
    <div class="mp-sub">
      Subtitle butuh model whisper{whisperModel.sizeBytes
        ? ` (~${formatBytes(whisperModel.sizeBytes)})`
        : ""}. Diunduh sekali dari Hugging Face, lalu disimpan di komputer ini —
      transkripsi tetap berjalan lokal.
    </div>
    {#if whisperModel.error}
      <div class="mp-error mono">{whisperModel.error}</div>
    {/if}
    <div class="mp-row mp-actions">
      {#if ondismiss}
        <button class="btn btn-ghost btn-sm" type="button" onclick={ondismiss}>Nanti Saja</button>
      {/if}
      <button class="btn btn-primary btn-sm" type="button" onclick={start}>
        {whisperModel.error ? "Coba Lagi" : "Unduh Model"}
      </button>
    </div>
  {/if}
</div>

<style>
  .model-prompt {
    display: grid;
    gap: 8px;
    padding: 12px 14px;
    background: hsl(213 94% 68% / 0.06);
    border: 1px solid hsl(213 94% 68% / 0.3);
    border-radius: var(--radius);
  }
  .mp-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--foreground);
  }
  .mp-sub {
    font-size: 12px;
    color: var(--muted);
  }
  .mp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .mp-actions {
    justify-content: flex-end;
  }
  .mp-error {
    font-size: 11px;
    color: var(--neg);
    word-break: break-word;
  }
  .mp-progress {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .mp-bar {
    flex: 1;
    height: 4px;
    border-radius: 999px;
    background: var(--input);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .mp-fill {
    height: 100%;
    background: var(--accent);
    transition: width 300ms ease;
  }
  .mp-pct {
    font-size: 10px;
    color: var(--muted-2);
    width: 32px;
    text-align: right;
  }
</style>
