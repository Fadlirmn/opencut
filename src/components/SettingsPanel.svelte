<script lang="ts">
  import { onMount } from "svelte";
  import { batch } from "../lib/batch.svelte";
  import { llmListModels } from "../lib/api";
  import { whisperModel } from "../lib/store.svelte";
  import type { ExportQuality, ExportResolution } from "../lib/types";
  import ModelDownloadPrompt from "./ModelDownloadPrompt.svelte";

  onMount(() => {
    void batch.refreshSettings();
    void batch.refreshLlmStatus();
    void whisperModel.refresh();
  });

  const qualityOptions: { value: ExportQuality; label: string }[] = [
    { value: "source", label: "asli" },
    { value: "high", label: "tinggi" },
    { value: "medium", label: "sedang" },
    { value: "small", label: "kecil" },
  ];
  const resolutionOptions: { value: ExportResolution; label: string }[] = [
    { value: "source", label: "source" },
    { value: "1080p", label: "1080p" },
    { value: "720p", label: "720p" },
    { value: "480p", label: "480p" },
  ];

  async function save() {
    if (batch.settings) await batch.saveSettings(batch.settings);
  }

  let authTab = $state<"api_key" | "oauth">("api_key");
  let apiKeyInput = $state("");

  async function connectApiKey() {
    await batch.setLlmApiKey(apiKeyInput);
    if (batch.llm.connected) apiKeyInput = "";
  }

  let modelOptions = $state<string[]>([]);
  let modelsBusy = $state(false);
  let modelsError = $state<string | null>(null);

  async function fetchModels() {
    if (!batch.settings) return;
    modelsBusy = true;
    modelsError = null;
    try {
      modelOptions = await llmListModels(batch.settings.llm.api_base, apiKeyInput || undefined);
    } catch (e) {
      modelsError = String(e);
    } finally {
      modelsBusy = false;
    }
  }
</script>

<section class="page">
  <header class="page-head">
    <h1>Settings</h1>
    <p>Pengaturan pemrosesan, integrasi LLM, dan nilai yang saat ini masih diatur lewat kode program.</p>
  </header>

  {#if batch.settingsError}
    <div class="error">{batch.settingsError}</div>
  {/if}

  {#if batch.settings}
    {@const s = batch.settings}
    <div class="grid">
      <!-- Pemrosesan -->
      <div class="card card-pad">
        <div class="card-head">
          <h2>Pemrosesan</h2>
          <p class="sub">default untuk setiap video yang masuk ke antrian</p>
        </div>

        <div class="switch-row">
          <div>
            <div class="s-title">Proses Paralel</div>
            <div class="s-sub">jalankan beberapa video bersamaan, bukan satu-per-satu</div>
          </div>
          <label class="switch">
            <input type="checkbox" bind:checked={s.parallel_enabled} />
            <span></span>
          </label>
        </div>

        {#if s.parallel_enabled}
          <div class="sub-panel">
            <div class="s-title" style="margin-bottom:8px;">Jumlah Video Bersamaan</div>
            <div class="count-row">
              {#each [1, 2, 3, 4] as n (n)}
                <button
                  class="count-btn"
                  class:active={s.parallel_count === n}
                  onclick={() => (s.parallel_count = n)}
                  type="button"
                >{n}</button>
              {/each}
            </div>
            <p class="tip muted-2">lebih banyak = lebih cepat, tapi lebih berat di CPU.</p>
          </div>
        {/if}

        <div class="divider"></div>

        <div class="switch-row">
          <div>
            <div class="s-title">Bakar Subtitle Bahasa Indonesia</div>
            <div class="s-sub">transkrip whisper dibakar ke video hasil export</div>
          </div>
          <label class="switch">
            <input type="checkbox" bind:checked={s.burn_subtitles} />
            <span></span>
          </label>
        </div>

        {#if s.burn_subtitles && whisperModel.status && !whisperModel.present}
          <ModelDownloadPrompt />
        {/if}

        <div class="divider"></div>

        <div class="opt">
          <div class="opt-label">Kualitas Export</div>
          <div class="seg">
            {#each qualityOptions as q (q.value)}
              <button class="seg-btn" class:active={s.quality === q.value} onclick={() => (s.quality = q.value)} type="button">
                {q.label}
              </button>
            {/each}
          </div>
        </div>

        <div class="opt">
          <div class="opt-label">Resolusi Export</div>
          <div class="seg">
            {#each resolutionOptions as r (r.value)}
              <button class="seg-btn" class:active={s.resolution === r.value} onclick={() => (s.resolution = r.value)} type="button">
                {r.label}
              </button>
            {/each}
          </div>
        </div>

        <div class="field">
          <label for="outputDir">Folder Output (Kosong = Sebelah File Asli)</label>
          <input id="outputDir" type="text" bind:value={s.output_dir} placeholder="~/Videos/autocut" />
        </div>
      </div>

      <!-- LLM -->
      <div class="card card-pad">
        <div class="card-head">
          <h2>Perbaiki Transkrip dengan LLM</h2>
          <p class="sub">opsional — hasil transkrip whisper dirapikan ejaan &amp; tanda bacanya oleh LLM yang kamu konfigurasi sendiri</p>
        </div>

        {#if batch.llm.connected}
          <div class="connected-row">
            <div class="connected-chip">
              <span class="dot"></span>
              <span class="c-name">
                Terhubung ({batch.llm.auth_mode === "api_key" ? "API Key" : "OAuth"}) ·
                {batch.llm.provider_label || "provider tersimpan"}
              </span>
            </div>
            <button class="btn btn-ghost btn-sm" onclick={() => batch.disconnectLlm()} type="button">Putuskan</button>
          </div>
          <div class="switch-row">
            <div>
              <div class="s-title">Rapikan Transkrip Otomatis</div>
              <div class="s-sub">jalan otomatis setelah transkripsi, sebelum dibakar ke subtitle</div>
            </div>
            <label class="switch">
              <input type="checkbox" bind:checked={s.llm.auto_rewrite} />
              <span></span>
            </label>
          </div>
          <button class="btn btn-ghost btn-sm" onclick={() => batch.testLlm()} type="button">Tes Koneksi</button>
          {#if batch.llmTestResult}
            <p class="tip" class:tip-ok={batch.llmTestResult === "berhasil"} class:tip-bad={batch.llmTestResult !== "berhasil"}>
              {batch.llmTestResult === "berhasil" ? "tes berhasil" : batch.llmTestResult}
            </p>
          {/if}
        {:else}
          <div class="opt">
            <div class="opt-label">Metode Autentikasi</div>
            <div class="seg">
              <button class="seg-btn" class:active={authTab === "api_key"} onclick={() => (authTab = "api_key")} type="button">
                API Key
              </button>
              <button class="seg-btn" class:active={authTab === "oauth"} onclick={() => (authTab = "oauth")} type="button">
                OAuth
              </button>
            </div>
          </div>

          <div class="field">
            <label for="providerLabel">Label Provider</label>
            <input id="providerLabel" type="text" bind:value={s.llm.provider_label} placeholder="mis. Provider Saya" />
          </div>
          <div class="field-row">
            <div class="field">
              <label for="apiBase">API Base (OpenAI-Compatible)</label>
              <input id="apiBase" type="text" bind:value={s.llm.api_base} placeholder="https://api.example.com/v1" />
            </div>
            <div class="field">
              <label for="model">Model</label>
              <div class="model-row">
                <input id="model" type="text" list="modelDatalist" bind:value={s.llm.model} placeholder="gpt-4o-mini" />
                <button
                  class="btn btn-ghost btn-sm"
                  disabled={modelsBusy || !s.llm.api_base.trim() || (authTab === "api_key" && !apiKeyInput.trim() && !batch.llm.connected)}
                  onclick={fetchModels}
                  type="button"
                >
                  {modelsBusy ? "…" : "Ambil Model"}
                </button>
              </div>
              <datalist id="modelDatalist">
                {#each modelOptions as m (m)}
                  <option value={m}></option>
                {/each}
              </datalist>
              {#if modelsError}
                <p class="tip tip-bad">{modelsError}</p>
              {:else if modelOptions.length > 0}
                <p class="tip">{modelOptions.length} model ditemukan — pilih dari dropdown field di atas</p>
              {/if}
            </div>
          </div>

          {#if authTab === "api_key"}
            <div class="field">
              <label for="apiKey">API Key</label>
              <input id="apiKey" type="password" autocomplete="off" bind:value={apiKeyInput} placeholder="sk-..." />
            </div>
            <button
              class="btn btn-primary"
              disabled={batch.llmBusy || !apiKeyInput.trim()}
              onclick={connectApiKey}
              type="button"
            >
              {batch.llmBusy ? "Menyimpan…" : "Simpan & Hubungkan"}
            </button>
          {:else}
            <div class="field-row">
              <div class="field">
                <label for="clientId">Client ID</label>
                <input id="clientId" type="text" bind:value={s.llm.client_id} />
              </div>
              <div class="field">
                <label for="scope">Scope</label>
                <input id="scope" type="text" bind:value={s.llm.scope} placeholder="openid profile" />
              </div>
            </div>
            <div class="field">
              <label for="authUrl">Authorization URL</label>
              <input id="authUrl" type="text" bind:value={s.llm.auth_url} placeholder="https://provider.example/oauth/authorize" />
            </div>
            <div class="field">
              <label for="tokenUrl">Token URL</label>
              <input id="tokenUrl" type="text" bind:value={s.llm.token_url} placeholder="https://provider.example/oauth/token" />
            </div>
            <button class="btn btn-primary" disabled={batch.llmBusy} onclick={() => batch.connectLlm()} type="button">
              {batch.llmBusy ? "Membuka Browser…" : "Hubungkan Akun (OAuth)"}
            </button>
            {#if batch.llmBusy}
              <button class="btn btn-ghost btn-sm" onclick={() => batch.cancelLlmConnect()} type="button">Batalkan</button>
            {/if}
          {/if}
          {#if batch.llmError}
            <p class="tip tip-bad">{batch.llmError}</p>
          {/if}
        {/if}

        <div class="divider"></div>
        <div class="info-banner">
          Pakai <strong>API Key</strong> buat provider custom/self-hosted (tinggal tempel key + API base) —
          atau <strong>OAuth</strong> kalau provider-nya butuh login akun; autocut tidak punya aplikasi OAuth
          terdaftar di provider manapun, jadi isi kredensial aplikasi OAuth milikmu sendiri. Hanya
          <strong>teks transkrip</strong> yang terkirim ke LLM; video tetap diproses 100% lokal. Key/token
          disimpan di keychain OS, bukan di file biasa.
        </div>
      </div>
    </div>

    <div class="save-row">
      <button class="btn btn-primary" disabled={batch.savingSettings} onclick={save} type="button">
        {batch.savingSettings ? "Menyimpan…" : "Simpan Pengaturan"}
      </button>
    </div>
  {/if}
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

  .error {
    padding: 10px 14px;
    background: hsl(0 84% 65% / 0.08);
    border: 1px solid hsl(0 84% 65% / 0.3);
    border-radius: var(--radius);
    color: var(--neg);
    font-size: 12px;
  }

  .grid {
    display: grid;
    gap: 16px;
  }
  @media (min-width: 760px) {
    .grid {
      grid-template-columns: 1fr 1fr;
    }
  }

  .card-head {
    margin-bottom: 14px;
  }
  .card-head h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--foreground);
  }
  .sub {
    margin: 2px 0 0;
    font-size: 11.5px;
    color: var(--muted-2);
  }

  .switch-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 4px 0;
  }
  .s-title {
    font-size: 13px;
    font-weight: 600;
  }
  .s-sub {
    font-size: 11px;
    color: var(--muted-2);
    margin-top: 2px;
  }
  .switch {
    position: relative;
    display: inline-flex;
    flex-shrink: 0;
  }
  .switch input {
    position: absolute;
    opacity: 0;
    width: 38px;
    height: 22px;
    margin: 0;
    cursor: pointer;
  }
  .switch span {
    width: 38px;
    height: 22px;
    border-radius: 999px;
    background: var(--input);
    border: 1px solid var(--border);
    position: relative;
    transition: background 140ms, border-color 140ms;
  }
  .switch span::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--muted-2);
    transition: transform 140ms, background 140ms;
  }
  .switch input:checked + span {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch input:checked + span::after {
    transform: translateX(16px);
    background: var(--primary-fg);
  }

  .divider {
    height: 1px;
    background: var(--border);
    margin: 14px 0;
  }
  .sub-panel {
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .count-row {
    display: flex;
    gap: 6px;
  }
  .count-btn {
    width: 34px;
    height: 30px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: 12px;
    cursor: pointer;
  }
  .count-btn.active {
    background: var(--accent);
    color: var(--primary-fg);
    border-color: var(--accent);
  }
  .tip {
    font-size: 11px;
    color: var(--muted-2);
    margin: 6px 0 0;
  }
  .tip-ok {
    color: var(--pos);
  }
  .tip-bad {
    color: var(--neg);
  }

  .opt {
    display: grid;
    gap: 6px;
    margin-bottom: 14px;
  }
  .opt-label {
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted-2);
  }
  .seg {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    background: var(--surface-2);
  }
  .seg-btn {
    height: 28px;
    background: transparent;
    border: 0;
    color: var(--muted);
    cursor: pointer;
    font: inherit;
    font-size: 11px;
    font-family: var(--font-mono);
  }
  .seg-btn + .seg-btn {
    border-left: 1px solid var(--border);
  }
  .seg-btn.active {
    background: var(--elevated);
    color: var(--foreground);
    box-shadow: inset 0 -2px 0 var(--accent);
  }

  .field {
    display: grid;
    gap: 6px;
    margin-bottom: 12px;
  }
  .field label {
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted-2);
  }
  .model-row {
    display: flex;
    gap: 6px;
  }
  .model-row input {
    flex: 1;
    min-width: 0;
  }
  .field-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .connected-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  .connected-chip {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius);
    background: hsl(142 71% 55% / 0.08);
    border: 1px solid hsl(142 71% 55% / 0.3);
    font-size: 12px;
  }
  .connected-chip .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--pos);
    flex-shrink: 0;
  }

  .info-banner {
    padding: 10px 12px;
    border-radius: var(--radius);
    background: hsl(280 70% 68% / 0.08);
    border: 1px solid hsl(280 70% 68% / 0.25);
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--muted);
  }

  .save-row {
    display: flex;
    justify-content: flex-end;
  }
</style>
