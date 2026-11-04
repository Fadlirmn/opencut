<script lang="ts">
  import { batch, type Page } from "../lib/batch.svelte";

  const items: { page: Page; label: string }[] = [
    { page: "upload", label: "upload & antrian" },
    { page: "log", label: "log" },
    { page: "settings", label: "settings" },
    { page: "profile", label: "profile" },
  ];

  let queueBadge = $derived(
    batch.jobs.filter((j) => j.state === "queued" || j.state === "running").length,
  );
</script>

<aside class="sidebar">
  <div class="brand">
    <span class="brand-dot"></span>
    <div class="brand-text">
      <div class="brand-name">autocut</div>
      <div class="brand-sub mono">desktop studio</div>
    </div>
  </div>

  <nav class="nav">
    {#each items as item (item.page)}
      <button
        class="nav-item"
        class:active={batch.page === item.page}
        onclick={() => (batch.page = item.page)}
        type="button"
      >
        <span>{item.label}</span>
        {#if item.page === "upload" && queueBadge > 0}
          <span class="badge mono">{queueBadge}</span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="foot">
    <div class="local-card">
      <span class="l-title">mode lokal</span>
      <span class="l-sub mono">windows · linux</span>
    </div>
    <p class="login-note">login akun belum tersedia untuk versi desktop ini — semua data tersimpan di komputer kamu.</p>
  </div>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--surface);
    border-right: 1px solid var(--border);
    min-width: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 16px 14px;
  }
  .brand-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 8px hsl(213 94% 68% / 0.6);
    flex-shrink: 0;
  }
  .brand-text {
    display: flex;
    flex-direction: column;
    line-height: 1.2;
  }
  .brand-name {
    font-weight: 600;
    font-size: 13px;
    color: var(--foreground);
  }
  .brand-sub {
    font-size: 9px;
    color: var(--muted-2);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 10px;
    overflow-y: auto;
  }
  .nav-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 9px 10px;
    border-radius: var(--radius-sm);
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: background 120ms, color 120ms;
  }
  .nav-item:hover {
    background: var(--surface-2);
    color: var(--foreground);
  }
  .nav-item.active {
    background: var(--elevated);
    color: var(--foreground);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--primary-fg);
    font-weight: 600;
  }

  .foot {
    padding: 12px;
    border-top: 1px solid var(--border);
  }
  .local-card {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .l-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--foreground);
  }
  .l-sub {
    font-size: 10px;
    color: var(--muted-2);
  }
  .login-note {
    font-size: 10px;
    color: var(--muted-2);
    line-height: 1.5;
    margin: 8px 2px 0;
  }
</style>
