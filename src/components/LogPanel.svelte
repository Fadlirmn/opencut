<script lang="ts">
  import { onMount } from "svelte";
  import { batch } from "../lib/batch.svelte";
  import { revealInFinder } from "../lib/api";

  onMount(() => {
    void batch.refreshLogs();
  });

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
            {#each filtered as entry (entry.finished_at_ms + entry.name)}
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
                  {#if entry.success && entry.output_path}
                    <button class="btn btn-ghost btn-sm" onclick={() => revealInFinder(entry.output_path!)} type="button">
                      lihat file
                    </button>
                  {/if}
                </td>
              </tr>
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
