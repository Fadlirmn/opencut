<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import icon from "../assets/icon.png";

  const appWindow = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    void appWindow.isMaximized().then((v) => (maximized = v));
    let unlisten: (() => void) | null = null;
    (async () => {
      unlisten = await appWindow.onResized(async () => {
        maximized = await appWindow.isMaximized();
      });
    })();
    return () => unlisten?.();
  });

  function minimize() {
    void appWindow.minimize();
  }
  function toggleMaximize() {
    void appWindow.toggleMaximize();
  }
  function closeWindow() {
    void appWindow.close();
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="tb-brand" data-tauri-drag-region>
    <img src={icon} alt="" class="tb-icon" />
    <span class="tb-title">Autocut</span>
  </div>

  <div class="tb-spacer" data-tauri-drag-region></div>

  <div class="tb-controls">
    <button class="tb-btn" onclick={minimize} aria-label="Minimize" type="button">
      <svg viewBox="0 0 10 10" width="10" height="10">
        <line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" stroke-width="1" />
      </svg>
    </button>
    <button class="tb-btn" onclick={toggleMaximize} aria-label="Maximize" type="button">
      {#if maximized}
        <svg viewBox="0 0 10 10" width="10" height="10">
          <rect x="1.5" y="2.5" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1" />
          <path d="M3.5 2.5 V1 H8.5 V6 H7" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
      {:else}
        <svg viewBox="0 0 10 10" width="10" height="10">
          <rect x="1" y="1" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
      {/if}
    </button>
    <button class="tb-btn tb-close" onclick={closeWindow} aria-label="Close" type="button">
      <svg viewBox="0 0 10 10" width="10" height="10">
        <line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" stroke-width="1" />
        <line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" stroke-width="1" />
      </svg>
    </button>
  </div>
</header>

<style>
  .titlebar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    user-select: none;
  }
  .tb-brand {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
    height: 100%;
  }
  .tb-icon {
    width: 15px;
    height: 15px;
    border-radius: 3px;
    flex-shrink: 0;
  }
  .tb-title {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--muted);
    letter-spacing: 0.02em;
  }
  .tb-spacer {
    flex: 1;
    height: 100%;
  }
  .tb-controls {
    display: flex;
    height: 100%;
    flex-shrink: 0;
  }
  .tb-btn {
    width: 44px;
    height: 100%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    transition: background 120ms, color 120ms;
  }
  .tb-btn:hover {
    background: var(--surface-2);
    color: var(--foreground);
  }
  .tb-close:hover {
    background: var(--neg);
    color: hsl(0 0% 100%);
  }
</style>
