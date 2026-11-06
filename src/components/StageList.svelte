<script lang="ts">
  import type { Stage } from "../lib/types";
  import { fmtDuration } from "../lib/clock";

  /// Per-stage progress: one row per pipeline step with its own bar,
  /// detail line, elapsed time and ETA. Used live in the queue and
  /// read-only in the log.
  let { stages }: { stages: Stage[] } = $props();

  function timing(s: Stage): string {
    if (s.state === "running") {
      const parts = [fmtDuration(s.elapsed_secs)];
      if (s.eta_secs != null) parts.push(`sisa ~${fmtDuration(s.eta_secs)}`);
      return parts.join(" · ");
    }
    if (s.state === "done" || s.state === "failed") return fmtDuration(s.elapsed_secs);
    return "";
  }
</script>

<ol class="stages">
  {#each stages as s (s.key)}
    <li class="stage {s.state}">
      <span class="dot" aria-hidden="true">
        {#if s.state === "done"}✓{:else if s.state === "failed"}✕{:else if s.state === "running"}●{:else}○{/if}
      </span>
      <div class="body">
        <div class="top">
          <span class="label">{s.label}</span>
          <span class="time mono">{timing(s)}</span>
        </div>
        {#if s.state === "running" || s.state === "failed"}
          <div class="bar-row">
            <div class="bar"><div class="fill" style:width="{s.pct}%"></div></div>
            <span class="pct mono">{s.pct.toFixed(0)}%</span>
          </div>
        {/if}
        {#if s.detail}
          <div class="detail mono" title={s.detail}>{s.detail}</div>
        {/if}
      </div>
    </li>
  {/each}
</ol>

<style>
  .stages {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .stage {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    font-size: 11.5px;
  }
  .dot {
    width: 14px;
    flex-shrink: 0;
    text-align: center;
    font-size: 10px;
    line-height: 16px;
    color: var(--muted-2);
  }
  .stage.done .dot {
    color: var(--pos, var(--accent));
  }
  .stage.running .dot {
    color: var(--accent);
  }
  .stage.failed .dot,
  .stage.failed .label {
    color: var(--neg);
  }
  .stage.pending {
    opacity: 0.55;
  }
  .body {
    flex: 1;
    min-width: 0;
  }
  .top {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    line-height: 16px;
  }
  .label {
    font-weight: 500;
  }
  .time {
    font-size: 10px;
    color: var(--muted-2);
    white-space: nowrap;
  }
  .bar-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 3px;
  }
  .bar {
    flex: 1;
    height: 4px;
    border-radius: 999px;
    background: var(--input);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 300ms ease;
  }
  .stage.failed .fill {
    background: var(--neg);
  }
  .pct {
    font-size: 10px;
    color: var(--muted-2);
    width: 32px;
    text-align: right;
  }
  .detail {
    font-size: 10px;
    color: var(--muted-2);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
