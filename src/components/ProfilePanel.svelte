<script lang="ts">
  import { diagnosticInfo } from "../lib/api";
  import { whisperModel } from "../lib/store.svelte";

  void whisperModel.refresh();

  let displayName = $state("");
  let saved = $state(false);
  let diag = $state<{ app_version: string; target_os: string; target_arch: string } | null>(null);

  try {
    displayName = localStorage.getItem("autocut.displayName") ?? "";
  } catch {
    /* storage may be unavailable; field stays empty */
  }

  void diagnosticInfo()
    .then((d) => (diag = d))
    .catch(() => {
      /* non-fatal: about card just shows placeholders */
    });

  function save() {
    try {
      localStorage.setItem("autocut.displayName", displayName);
    } catch {
      /* non-critical per-device convenience; nothing to recover */
    }
    saved = true;
    setTimeout(() => (saved = false), 1800);
  }
</script>

<section class="page">
  <header class="page-head">
    <h1>Profile</h1>
    <p>Preferensi lokal aplikasi — tidak ada akun online yang tersambung.</p>
  </header>

  <div class="info-banner">
    Login akun belum tersedia di versi Windows &amp; Linux ini. Nama di bawah hanya dipakai
    untuk menandai export dan log di komputer ini — fitur akun online akan ditambahkan menyusul.
  </div>

  <div class="card card-pad">
    <div class="card-head">
      <h2>Identitas Lokal</h2>
    </div>
    <div class="field">
      <label for="displayName">Nama Tampilan</label>
      <input id="displayName" type="text" bind:value={displayName} />
    </div>
    <div class="row-between">
      <span class="muted-2 tip">perubahan disimpan di perangkat ini</span>
      <button class="btn btn-primary btn-sm" onclick={save} type="button">
        {saved ? "Tersimpan ✓" : "Simpan"}
      </button>
    </div>
  </div>

  <div class="card card-pad">
    <div class="card-head">
      <h2>Tentang Aplikasi</h2>
      <p class="sub">autocut berjalan sepenuhnya lokal — tidak ada video yang diunggah ke server</p>
    </div>
    <div class="about-grid">
      <div class="about-item">
        <div class="v mono">{diag?.app_version ?? "—"}</div>
        <div class="k">Versi</div>
      </div>
      <div class="about-item">
        <div class="v mono">{diag ? `${diag.target_os}/${diag.target_arch}` : "—"}</div>
        <div class="k">Platform</div>
      </div>
      <div class="about-item">
        <div class="v mono" title={whisperModel.status?.path ?? ""}>
          large-v3-turbo{whisperModel.status
            ? whisperModel.present
              ? ""
              : " (belum diunduh)"
            : ""}
        </div>
        <div class="k">Model Subtitle</div>
      </div>
    </div>
  </div>
</section>

<style>
  .page {
    display: grid;
    gap: 16px;
    max-width: 680px;
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

  .info-banner {
    padding: 12px 14px;
    border-radius: var(--radius);
    background: hsl(213 94% 68% / 0.08);
    border: 1px solid hsl(213 94% 68% / 0.25);
    font-size: 12px;
    line-height: 1.55;
    color: var(--muted);
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

  .field {
    display: grid;
    gap: 6px;
    margin-bottom: 14px;
  }
  .field label {
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted-2);
  }

  .row-between {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .tip {
    font-size: 11px;
  }

  .about-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
  }
  .about-item {
    padding: 12px;
    text-align: center;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .about-item .v {
    font-size: 14px;
    font-weight: 600;
    color: var(--foreground);
  }
  .about-item .k {
    font-size: 10px;
    color: var(--muted-2);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-top: 3px;
  }

  @media (max-width: 480px) {
    .about-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
