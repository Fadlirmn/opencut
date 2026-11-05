<script lang="ts">
  // The single-video editor (Dropzone/VideoPlayer/Timeline/ParameterPanel/
  // ManualCutPanel/ExportPanel/ResizableSplit) is hidden from the primary
  // UI, not deleted — the batch queue below is now the main workflow.
  // Those components and their store (lib/store.svelte.ts) stay in the repo
  // for whoever wants to bring the manual-editing mode back later.
  import { onMount } from "svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import UploadQueuePanel from "./components/UploadQueuePanel.svelte";
  import LogPanel from "./components/LogPanel.svelte";
  import SettingsPanel from "./components/SettingsPanel.svelte";
  import ProfilePanel from "./components/ProfilePanel.svelte";
  import { batch } from "./lib/batch.svelte";

  onMount(() => {
    void batch.init();
    return () => batch.dispose();
  });
</script>

<div class="app-root">
  <TitleBar />
  <main class="shell">
    <Sidebar />
    <section class="main">
      {#if batch.page === "upload"}
        <UploadQueuePanel />
      {:else if batch.page === "log"}
        <LogPanel />
      {:else if batch.page === "settings"}
        <SettingsPanel />
      {:else}
        <ProfilePanel />
      {/if}
    </section>
  </main>
</div>

<style>
  .app-root {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .shell {
    display: grid;
    grid-template-columns: 220px 1fr;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .main {
    overflow-y: auto;
    min-width: 0;
    background: var(--background);
  }

  @media (max-width: 720px) {
    .shell {
      grid-template-columns: 1fr;
    }
  }
</style>
