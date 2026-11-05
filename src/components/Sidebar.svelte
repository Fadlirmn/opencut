<script lang="ts">
  import { batch, type Page } from "../lib/batch.svelte";
  import icon from "../assets/icon.png";

  const items: { page: Page; label: string }[] = [
    { page: "upload", label: "Upload & Antrian" },
    { page: "log", label: "Log" },
    { page: "settings", label: "Settings" },
    { page: "profile", label: "Profile" },
  ];

  let queueBadge = $derived(
    batch.jobs.filter((j) => j.state === "queued" || j.state === "running").length,
  );
</script>

<aside class="sidebar">
  <div class="brand">
    <img src={icon} alt="" class="brand-icon" />
    <div class="brand-text">
      <div class="brand-name">Autocut</div>
      <div class="brand-sub mono">Desktop Studio</div>
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
        <span class="nav-left">
          <svg class="nav-icon" viewBox="0 0 16 16" width="16" height="16" fill="none">
            {#if item.page === "upload"}
              <path d="M8 10.5V2.5M8 2.5 4.5 6M8 2.5 11.5 6" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
              <path d="M2.5 11v1.5A1.5 1.5 0 0 0 4 14h8a1.5 1.5 0 0 0 1.5-1.5V11" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
            {:else if item.page === "log"}
              <path d="M8 4.5v4l2.5 1.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
              <circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.4" />
            {:else if item.page === "settings"}
              <circle cx="8" cy="8" r="2" stroke="currentColor" stroke-width="1.4" />
              <path d="M8 2v1.3M8 12.7V14M14 8h-1.3M3.3 8H2M12.1 3.9l-.9.9M4.8 11.1l-.9.9M12.1 12.1l-.9-.9M4.8 4.9l-.9-.9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            {:else}
              <circle cx="8" cy="5.5" r="2.5" stroke="currentColor" stroke-width="1.4" />
              <path d="M3 13.5c0-2.5 2.2-4 5-4s5 1.5 5 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            {/if}
          </svg>
          <span>{item.label}</span>
        </span>
        {#if item.page === "upload" && queueBadge > 0}
          <span class="badge mono">{queueBadge}</span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="foot">
    <div class="local-card">
      <span class="l-title">Mode Lokal</span>
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
  .brand-icon {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
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
  .nav-left {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }
  .nav-icon {
    flex-shrink: 0;
    color: inherit;
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
