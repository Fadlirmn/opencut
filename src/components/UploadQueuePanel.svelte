<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open } from "@tauri-apps/plugin-dialog";
  import { batch } from "../lib/batch.svelte";
  import { openWithDefaultApp } from "../lib/api";
  import ModelDownloadPrompt from "./ModelDownloadPrompt.svelte";
  import StageList from "./StageList.svelte";

  const VIDEO_RE = /\.(mp4|mov|m4v|mkv|webm|avi)$/i;

  let hovering = $state(false);
  let processOpen = $state(true);
  let doneOpen = $state(true);
  let starting = $state(false);
  /// Finished jobs whose stage breakdown is expanded.
  let expanded = $state<Record<number, boolean>>({});

  onMount(() => {
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const webview = getCurrentWebview();
        unlisten = await webview.onDragDropEvent((event) => {
          if (event.payload.type === "over") {
            hovering = true;
          } else if (event.payload.type === "leave") {
            hovering = false;
          } else if (event.payload.type === "drop") {
            hovering = false;
            const accepted = event.payload.paths.filter((p) => VIDEO_RE.test(p));
            if (accepted.length) void batch.addFiles(accepted);
          }
        });
      } catch {
        /* browser preview lacks Tauri's webview drag/drop API */
      }
    })();
    return () => unlisten?.();
  });

  async function browseFiles() {
    const selected = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "m4v", "mkv", "webm", "avi"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    await batch.addFiles(paths);
  }

  function basename(p: string): string {
    return p.split(/[/\\]/).pop() ?? p;
  }

  let processingJobs = $derived(
    batch.jobs.filter((j) => j.state === "queued" || j.state === "running"),
  );
  let finishedJobs = $derived(
    batch.jobs.filter((j) => j.state === "done" || j.state === "failed"),
  );
  let runCount = $derived(processingJobs.filter((j) => j.state === "running").length);
  let waitCount = $derived(processingJobs.filter((j) => j.state === "queued").length);
  let doneCount = $derived(finishedJobs.filter((j) => j.state === "done").length);
  let failCount = $derived(finishedJobs.filter((j) => j.state === "failed").length);

  async function startQueue() {
    starting = true;
    try {
      await batch.start();
    } finally {
      starting = false;
    }
  }
</script>

<section class="page">
  <header class="page-head">
    <h1>Upload &amp; Antrian</h1>
    <p>
      Tambahkan beberapa video sekaligus — diproses satu per satu (atau paralel,
      lihat Settings). Hening dipotong otomatis dan subtitle dibakar kalau diaktifkan.
    </p>
  </header>

  <div class="dropzone card" class:hovering>
    <div class="dz-title mono">Drop Video di Sini</div>
    <div class="dz-sub">atau pilih beberapa file sekaligus dari komputer</div>
    <button class="btn btn-primary" type="button" onclick={browseFiles}>+ Tambah Video</button>
    <div class="formats mono">mp4 · mov · m4v · mkv · webm · avi</div>
  </div>

  <section class="card queue-card" class:collapsed={!processOpen}>
    <header
      class="queue-head"
      onclick={() => (processOpen = !processOpen)}
      role="button"
      tabindex="0"
      onkeydown={(e) => e.key === "Enter" && (processOpen = !processOpen)}
    >
      <span class="chev" class:closed={!processOpen}>▾</span>
      <div class="htxt">
        <h2>Sedang Diproses</h2>
        <span class="hint muted-2">{runCount} memproses · {waitCount} menunggu</span>
      </div>
      <span class="chip mono">{processingJobs.length} video</span>
    </header>
    <div class="queue-list">
      {#if processingJobs.length === 0}
        <p class="empty muted-2">Tidak ada video dalam antrian — tambahkan di atas.</p>
      {:else}
        {#each processingJobs as job (job.id)}
          <div class="q-item">
          <div class="q-row">
            <div class="q-info">
              <div class="q-name" title={job.path}>{job.name}</div>
              <div class="q-meta mono muted-2">{job.message}</div>
            </div>
            {#if job.state === "running"}
              <div class="q-progress">
                <div class="q-progress-bar">
                  <div class="q-progress-fill" style:width="{job.pct}%"></div>
                </div>
                <span class="q-pct mono">{job.pct.toFixed(0)}%</span>
              </div>
            {:else}
              <span class="pill pill-wait mono">menunggu</span>
            {/if}
            <button
              class="icon-btn"
              title="Hapus dari antrian (membatalkan kalau sedang diproses)"
              onclick={() => batch.remove(job.id)}
              type="button"
            >×</button>
          </div>
          {#if job.state === "running" && job.stages.length}
            <div class="q-stages"><StageList stages={job.stages} /></div>
          {/if}
          </div>
        {/each}
      {/if}
    </div>
    <footer class="queue-foot">
      <span class="muted-2 mono">{runCount} memproses · {waitCount} menunggu</span>
      <button
        class="btn btn-primary btn-sm"
        disabled={starting || processingJobs.length === 0}
        onclick={startQueue}
        type="button"
      >{starting ? "Memulai…" : "Proses Semua"}</button>
    </footer>
    {#if batch.modelNeeded}
      <div class="queue-notice">
        <ModelDownloadPrompt
          onready={startQueue}
          ondismiss={() => (batch.modelNeeded = false)}
        />
      </div>
    {:else if batch.startError}
      <div class="queue-notice">
        <p class="start-error mono">{batch.startError}</p>
      </div>
    {/if}
  </section>

  <section class="card queue-card" class:collapsed={!doneOpen}>
    <header
      class="queue-head"
      onclick={() => (doneOpen = !doneOpen)}
      role="button"
      tabindex="0"
      onkeydown={(e) => e.key === "Enter" && (doneOpen = !doneOpen)}
    >
      <span class="chev" class:closed={!doneOpen}>▾</span>
      <div class="htxt">
        <h2>Selesai</h2>
        <span class="hint muted-2">{doneCount} berhasil{failCount ? ` · ${failCount} gagal` : ""}</span>
      </div>
      <span class="chip mono">{finishedJobs.length} video</span>
    </header>
    <div class="queue-list">
      {#if finishedJobs.length === 0}
        <p class="empty muted-2">Belum ada video yang selesai diproses.</p>
      {:else}
        {#each finishedJobs as job (job.id)}
          <div class="q-item">
          <div class="q-row">
            <div class="q-info">
              <div class="q-name" title={job.path}>{job.name}</div>
              {#if job.state === "failed"}
                <div class="q-meta mono error-text" title={job.error ?? ""}>{job.error}</div>
              {:else}
                <div class="q-meta mono muted-2" title={job.output_path ?? ""}>
                  {job.output_path ? basename(job.output_path) : ""}
                </div>
              {/if}
            </div>
            {#if job.state === "done"}
              <span class="pill pill-done mono">selesai</span>
              {#if job.output_path}
                <button class="btn btn-ghost btn-sm" onclick={() => openWithDefaultApp(job.output_path!)} type="button">
                  Preview
                </button>
              {/if}
            {:else}
              <span class="pill pill-fail mono">gagal</span>
              {#if job.stages.length}
                <button
                  class="btn btn-ghost btn-sm"
                  onclick={() => (expanded[job.id] = !expanded[job.id])}
                  type="button"
                >{expanded[job.id] ? "Tutup" : "Detail"}</button>
              {/if}
              <button class="btn btn-ghost btn-sm" onclick={() => batch.retry(job.id)} type="button">
                Coba Lagi
              </button>
            {/if}
            <button class="icon-btn" title="Hapus" onclick={() => batch.remove(job.id)} type="button">×</button>
          </div>
          {#if expanded[job.id] && job.stages.length}
            <div class="q-stages">
              <StageList stages={job.stages} />
              {#if job.error}
                <pre class="q-error mono">{job.error}</pre>
              {/if}
            </div>
          {/if}
          </div>
        {/each}
      {/if}
    </div>
    <footer class="queue-foot">
      <span></span>
      <button
        class="btn btn-ghost btn-sm"
        disabled={doneCount === 0}
        onclick={() => batch.clearDone()}
        type="button"
      >Hapus yang Selesai</button>
    </footer>
  </section>
</section>

<style>
  .page {
    display: grid;
    gap: 16px;
    max-width: 920px;
    padding: 24px 28px 40px;
  }
  .page-head h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .page-head p {
    margin: 4px 0 0;
    font-size: 13px;
    color: var(--muted);
    max-width: 65ch;
  }

  .dropzone {
    padding: 28px 24px;
    text-align: center;
    display: grid;
    gap: 10px;
    place-items: center;
    border-style: dashed;
    border-color: var(--border-strong);
    border-width: 1.5px;
    background: linear-gradient(180deg, hsl(240 6% 8%) 0%, hsl(240 6% 6%) 100%);
    transition: border-color 160ms, background 160ms;
  }
  .dropzone.hovering {
    border-color: var(--accent);
    background: radial-gradient(circle at 50% 0%, hsl(213 94% 68% / 0.12), transparent 60%), hsl(240 6% 8%);
  }
  .dz-title {
    font-size: 16px;
    font-weight: 500;
  }
  .dz-sub {
    font-size: 12px;
    color: var(--muted-2);
  }
  .formats {
    font-size: 11px;
    color: var(--muted-2);
    letter-spacing: 0.04em;
    margin-top: 2px;
  }

  .queue-card {
    overflow: hidden;
  }
  .queue-head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    cursor: pointer;
  }
  .queue-head:hover {
    background: var(--elevated);
  }
  .chev {
    color: var(--muted-2);
    transition: transform 160ms;
    flex-shrink: 0;
  }
  .chev.closed {
    transform: rotate(-90deg);
  }
  .htxt {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .htxt h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--foreground);
  }
  .hint {
    font-size: 11px;
  }
  .queue-card.collapsed .queue-list,
  .queue-card.collapsed .queue-foot,
  .queue-card.collapsed .hint {
    display: none;
  }
  .chip {
    font-size: 10.5px;
    padding: 2px 9px;
    border-radius: 999px;
    background: var(--input);
    border: 1px solid var(--border);
    color: var(--muted);
  }

  .queue-list {
    max-height: 360px;
    overflow-y: auto;
    padding: 4px;
  }
  .empty {
    padding: 24px 16px;
    text-align: center;
    font-size: 12px;
  }
  .q-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 10px;
    border-radius: var(--radius-sm);
  }
  .q-item {
    display: flex;
    flex-direction: column;
  }
  .q-stages {
    padding: 4px 14px 12px 14px;
  }
  .q-error {
    margin: 8px 0 0;
    padding: 8px 10px;
    font-size: 10.5px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 220px;
    overflow: auto;
    color: var(--neg);
    background: var(--input);
    border: 1px solid var(--border);
    border-radius: 6px;
    user-select: text;
  }
  .q-row:hover {
    background: var(--surface-2);
  }
  .q-info {
    flex: 1;
    min-width: 0;
  }
  .q-name {
    font-size: 12.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .q-meta {
    font-size: 10.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error-text {
    color: var(--neg);
  }

  .q-progress {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 160px;
    flex-shrink: 0;
  }
  .q-progress-bar {
    flex: 1;
    height: 4px;
    border-radius: 999px;
    background: var(--input);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .q-progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 300ms ease;
  }
  .q-pct {
    font-size: 10px;
    color: var(--muted-2);
    width: 32px;
    text-align: right;
  }

  .pill {
    font-size: 10px;
    font-weight: 600;
    padding: 3px 9px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .pill-wait {
    background: var(--input);
    color: var(--muted-2);
    border: 1px solid var(--border);
  }
  .pill-done {
    background: hsl(142 71% 55% / 0.12);
    color: var(--pos);
    border: 1px solid hsl(142 71% 55% / 0.3);
  }
  .pill-fail {
    background: hsl(0 84% 65% / 0.12);
    color: var(--neg);
    border: 1px solid hsl(0 84% 65% / 0.3);
  }

  .icon-btn {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--muted);
    cursor: pointer;
    flex-shrink: 0;
    transition: all 120ms;
  }
  .icon-btn:hover {
    color: var(--neg);
    border-color: hsl(0 84% 65% / 0.4);
  }

  .queue-notice {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
  }
  .queue-card.collapsed .queue-notice {
    display: none;
  }
  .start-error {
    margin: 0;
    font-size: 11px;
    color: var(--neg);
    word-break: break-word;
  }

  .queue-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 16px;
    background: var(--surface-2);
    border-top: 1px solid var(--border);
  }
</style>
