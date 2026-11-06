<script lang="ts">
  import { onMount } from "svelte";
  import { batch } from "../lib/batch.svelte";
  import { revealInFinder, openWithDefaultApp } from "../lib/api";
  import type { LogEntry } from "../lib/types";
  import { fmtDuration } from "../lib/clock";
  import StageList from "./StageList.svelte";

  /// Rows whose detail panel is open, keyed by entryKey().
  let open = $state<Record<string, boolean>>({});
  let copied = $state<string | null>(null);

  function hasDetail(e: LogEntry): boolean {
    return !!(e.error || e.stages?.length || e.source_path);
  }

  /// Plain-text report of one run, for pasting into a bug report.
  function reportText(e: LogEntry): string {
    const lines = [
      `file: ${e.source_path ?? e.name}`,
      `status: ${e.success ? "berhasil" : "gagal"}`,
      `selesai: ${fmtDate(e.finished_at_ms)}`,
    ];
    if (e.duration_secs != null) lines.push(`durasi: ${fmtDuration(e.duration_secs)}`);
    if (e.app_version) lines.push(`versi: ${e.app_version}`);
    if (e.os) lines.push(`os: ${e.os}`);
    lines.push(`subtitle: ${e.subtitles ? "ya" : "tidak"}`);
    if (e.failed_stage) lines.push(`tahap gagal: ${e.failed_stage}`);
    for (const s of e.stages ?? []) {
      const t = s.state === "pending" ? "-" : fmtDuration(s.elapsed_secs);
      lines.push(`  [${s.state}] ${s.label} (${t})${s.detail ? ` — ${s.detail}` : ""}`);
    }
    if (e.output_path) lines.push(`output: ${e.output_path}`);
    if (e.error) lines.push("", "error:", e.error);
    return lines.join("\n");
  }

  async function copyReport(e: LogEntry) {
    try {
      await navigator.clipboard.writeText(reportText(e));
      copied = entryKey(e);
      setTimeout(() => (copied = null), 1500);
    } catch {
      /* clipboard can be unavailable; the text is still selectable below */
    }
  }

  onMount(() => {
    void batch.refreshLogs();
  });

  function entryKey(e: { finished_at_ms: number; name: string }): string {
    return e.finished_at_ms + e.name;
  }

  type Filter = "semua" | "berhasil" | "gagal";
  let filter = $state<Filter>("semua");

  let filtered = $derived(
    batch.logs.filter((e) => {
      if (filter === "berhasil") return e.success;
      if (filter === "gagal") return !e.success;
      return true;
    }),
  );
  let successCount = $derived(batch.logs.filter((e) => e.success).length);
  let failCount = $derived(batch.logs.filter((e) => !e.success).length);

  function fmtDate(ms: number): string {
    try {
      return new Date(ms).toLocaleString();
    } catch {
      return "-";
    }
  }
</script>

<section class="page">
  <header class="page-head">
    <h1>Log</h1>
    <p>Riwayat semua video yang sudah diproses di komputer ini.</p>
  </header>

  <div class="filter-row">
    <button class="filter-btn" class:active={filter === "semua"} onclick={() => (filter = "semua")} type="button">
      Semua ({batch.logs.length})
    </button>
    <button class="filter-btn" class:active={filter === "berhasil"} onclick={() => (filter = "berhasil")} type="button">
      Berhasil ({successCount})
    </button>
    <button class="filter-btn" class:active={filter === "gagal"} onclick={() => (filter = "gagal")} type="button">
      Gagal ({failCount})
    </button>
  </div>

  {#if batch.logsError}
    <div class="error">{batch.logsError}</div>
  {/if}

  <div class="card table-card">
    {#if filtered.length === 0}
      <p class="empty muted-2">Belum ada log untuk filter ini.</p>
    {:else}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>File</th>
              <th>Selesai</th>
              <th>Subtitle</th>
              <th>Status</th>
              <th style="text-align:right">Aksi</th>
            </tr>
          </thead>
          <tbody>
            {#each filtered as entry (entryKey(entry))}
              <tr>
                <td class="mono">{entry.name}</td>
                <td class="muted">{fmtDate(entry.finished_at_ms)}</td>
                <td>
                  {#if entry.subtitles}
                    <span class="pill pill-done mono">id</span>
                  {:else}
                    <span class="mono muted-2">—</span>
                  {/if}
                </td>
                <td>
                  {#if entry.success}
                    <span class="pill pill-done mono">berhasil</span>
                  {:else}
                    <span class="pill pill-fail mono" title={entry.error ?? ""}>gagal</span>
                  {/if}
                </td>
                <td style="text-align:right">
                  {#if hasDetail(entry)}
                    <button
                      class="btn btn-ghost btn-sm"
                      onclick={() => (open[entryKey(entry)] = !open[entryKey(entry)])}
                      type="button"
                    >{open[entryKey(entry)] ? "Tutup" : "Detail"}</button>
                  {/if}
                  {#if entry.success && entry.output_path}
                    <button class="btn btn-ghost btn-sm" onclick={() => openWithDefaultApp(entry.output_path!)} type="button">
                      Preview
                    </button>
                    <button class="btn btn-ghost btn-sm" onclick={() => revealInFinder(entry.output_path!)} type="button">
                      Lihat File
                    </button>
                  {/if}
                </td>
              </tr>
              {#if open[entryKey(entry)]}
                <tr class="detail-row">
                  <td colspan="5">
                    <div class="detail">
                      <dl class="meta">
                        {#if entry.source_path}<dt>File</dt><dd class="mono">{entry.source_path}</dd>{/if}
                        {#if entry.duration_secs != null}<dt>Durasi</dt><dd>{fmtDuration(entry.duration_secs)}</dd>{/if}
                        {#if entry.failed_stage}<dt>Tahap gagal</dt><dd class="neg">{entry.failed_stage}</dd>{/if}
                        {#if entry.output_path}<dt>Output</dt><dd class="mono">{entry.output_path}</dd>{/if}
                        {#if entry.app_version || entry.os}
                          <dt>Versi</dt><dd class="mono">{entry.app_version ?? "?"} · {entry.os ?? "?"}</dd>
                        {/if}
                      </dl>
                      {#if entry.stages?.length}
                        <StageList stages={entry.stages} />
                      {/if}
                      {#if entry.error}
                        <pre class="err mono">{entry.error}</pre>
                      {/if}
                      <div class="detail-actions">
                        <button class="btn btn-ghost btn-sm" onclick={() => copyReport(entry)} type="button">
                          {copied === entryKey(entry) ? "Tersalin ✓" : "Salin Laporan"}
                        </button>
                      </div>
                    </div>
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
      <footer class="table-foot">
        <span>menampilkan {filtered.length} dari {batch.logs.length} log</span>
        <span>tersimpan lokal di komputer ini</span>
      </footer>
    {/if}
  </div>
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
  }

  .filter-row {
    display: flex;
    gap: 8px;
  }
  .filter-btn {
    padding: 6px 13px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--muted);
    transition: all 120ms;
  }
  .filter-btn.active {
    background: var(--foreground);
    color: var(--background);
    border-color: var(--foreground);
  }
  .filter-btn:hover:not(.active) {
    color: var(--foreground);
    border-color: var(--border-strong);
  }

  .error {
    padding: 10px 14px;
    background: hsl(0 84% 65% / 0.08);
    border: 1px solid hsl(0 84% 65% / 0.3);
    border-radius: var(--radius);
    color: var(--neg);
    font-size: 12px;
  }

  .detail-row td {
    background: var(--surface-2);
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 6px 4px;
    text-align: left;
  }
  .meta {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 3px 12px;
    margin: 0;
    font-size: 11.5px;
  }
  .meta dt {
    color: var(--muted-2);
  }
  .meta dd {
    margin: 0;
    word-break: break-all;
    user-select: text;
  }
  .meta .neg {
    color: var(--neg);
  }
  .err {
    margin: 0;
    padding: 8px 10px;
    font-size: 10.5px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 260px;
    overflow: auto;
    color: var(--neg);
    background: var(--input);
    border: 1px solid var(--border);
    border-radius: 6px;
    user-select: text;
  }
  .detail-actions {
    display: flex;
    justify-content: flex-end;
  }

  .table-card {
    overflow: hidden;
  }
  .empty {
    padding: 40px 16px;
    text-align: center;
    font-size: 12px;
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  thead tr {
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }
  th {
    text-align: left;
    padding: 10px 14px;
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted-2);
    font-weight: 500;
    white-space: nowrap;
  }
  tbody tr {
    border-bottom: 1px solid var(--border);
  }
  tbody tr:last-child {
    border-bottom: 0;
  }
  tbody tr:hover {
    background: var(--surface-2);
  }
  td {
    padding: 10px 14px;
    font-size: 12.5px;
    white-space: nowrap;
  }

  .pill {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
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

  .table-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: var(--surface-2);
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--muted-2);
  }
</style>
