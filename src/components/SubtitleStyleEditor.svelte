<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { openVideo } from "../lib/api";
  import { batch } from "../lib/batch.svelte";
  import {
    DEFAULT_SUBTITLE_STYLE,
    rgba,
    subtitleMetrics,
    wrapCaption,
    type SubtitleStyle,
  } from "../lib/subtitle";
  import Slider from "./Slider.svelte";

  let { style = $bindable() }: { style: SubtitleStyle } = $props();

  // ---- preview source -------------------------------------------------
  // A real frame from the user's own footage when we can get one (picked
  // here, or the most recent video in the queue), otherwise a neutral frame
  // at a chosen aspect ratio.
  type Frame = { width: number; height: number; src: string | null; name: string | null };
  const ASPECTS = {
    portrait: { width: 1080, height: 1920, label: "9:16" },
    landscape: { width: 1920, height: 1080, label: "16:9" },
    square: { width: 1080, height: 1080, label: "1:1" },
  } as const;
  type Aspect = keyof typeof ASPECTS;

  let aspect = $state<Aspect>("portrait");
  let video = $state<Frame | null>(null);
  let videoError = $state<string | null>(null);
  let frame = $derived<Frame>(
    video ?? { ...ASPECTS[aspect], src: null, name: null },
  );

  async function useVideo(path: string) {
    videoError = null;
    try {
      const info = await openVideo(path);
      const swap = info.rotation === 90 || info.rotation === 270;
      video = {
        width: swap ? info.height : info.width,
        height: swap ? info.width : info.height,
        src: convertFileSrc(path),
        name: path.split(/[/\\]/).pop() ?? path,
      };
    } catch (e) {
      videoError = String(e);
    }
  }

  async function pickVideo() {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "m4v", "mkv", "webm", "avi"] }],
    });
    if (typeof picked === "string") await useVideo(picked);
  }

  onMount(() => {
    const recent = batch.jobs.at(-1);
    if (recent) void useVideo(recent.path);
  });

  function seekToMiddle(e: Event) {
    const el = e.currentTarget as HTMLVideoElement;
    if (Number.isFinite(el.duration)) el.currentTime = el.duration / 3;
  }

  // ---- preview geometry ----------------------------------------------
  // The preview box is the frame scaled down; every pixel value from the
  // shared metrics is multiplied by the same factor.
  let boxWidth = $state(0);
  let scale = $derived(boxWidth > 0 ? boxWidth / frame.width : 0);
  let metrics = $derived(subtitleMetrics(frame.width, frame.height, style));

  let sample = $state("Ini contoh subtitle yang bakal muncul di video kamu");
  let lines = $derived(
    wrapCaption(style.uppercase ? sample.toUpperCase() : sample, metrics.maxChars),
  );

  let boxPad = $derived(
    style.background ? Math.max(metrics.outline, metrics.fontSize * 0.15) : 0,
  );

  // ---- presets --------------------------------------------------------
  const presets: { name: string; style: Partial<SubtitleStyle> }[] = [
    { name: "Klasik", style: { text_color: "#FFFFFF", outline_color: "#000000", outline: 5, background: false, bold: true } },
    { name: "Kuning", style: { text_color: "#FFE600", outline_color: "#000000", outline: 6, background: false, bold: true } },
    { name: "Kotak Hitam", style: { text_color: "#FFFFFF", outline_color: "#000000", background: true, background_opacity: 65, bold: true } },
    { name: "Kotak Putih", style: { text_color: "#111111", outline_color: "#FFFFFF", background: true, background_opacity: 90, bold: true } },
  ];

  function applyPreset(p: Partial<SubtitleStyle>) {
    Object.assign(style, p);
  }
  function reset() {
    Object.assign(style, DEFAULT_SUBTITLE_STYLE);
  }
</script>

<div class="editor">
  <div class="controls">
    <div class="opt">
      <div class="opt-label">Preset</div>
      <div class="presets">
        {#each presets as p (p.name)}
          <button class="btn btn-sm" type="button" onclick={() => applyPreset(p.style)}>{p.name}</button>
        {/each}
        <button class="btn btn-ghost btn-sm" type="button" onclick={reset}>Reset</button>
      </div>
    </div>

    <Slider bind:value={style.size_pct} min={3} max={15} step={0.5} label="ukuran" format={(v) => `${v.toFixed(1)}%`} />
    <Slider bind:value={style.position_pct} min={0} max={60} step={1} label="jarak dari bawah" format={(v) => `${v.toFixed(0)}%`} />
    {#if !style.background}
      <Slider bind:value={style.outline} min={0} max={15} step={0.5} label="tebal garis tepi" format={(v) => v.toFixed(1)} />
    {:else}
      <Slider bind:value={style.background_opacity} min={10} max={100} step={5} label="gelap kotak" format={(v) => `${v.toFixed(0)}%`} />
    {/if}
    <Slider
      bind:value={style.max_chars_per_line}
      min={0}
      max={60}
      step={1}
      label="huruf per baris"
      format={(v) => (v === 0 ? `auto (${metrics.maxChars})` : v.toFixed(0))}
    />

    <div class="colors">
      <label class="color">
        <input type="color" bind:value={style.text_color} />
        <span>Warna teks</span>
      </label>
      <label class="color">
        <input type="color" bind:value={style.outline_color} />
        <span>{style.background ? "Warna kotak" : "Warna garis tepi"}</span>
      </label>
    </div>

    <div class="toggles">
      <label class="check"><input type="checkbox" bind:checked={style.bold} /> Tebal</label>
      <label class="check"><input type="checkbox" bind:checked={style.uppercase} /> HURUF BESAR</label>
      <label class="check"><input type="checkbox" bind:checked={style.background} /> Kotak latar</label>
    </div>

    <div class="field">
      <label for="subSample">Teks Contoh</label>
      <input id="subSample" type="text" bind:value={sample} />
    </div>
  </div>

  <div class="preview-col">
    <div class="preview-head">
      {#if video}
        <span class="mono muted-2 vname" title={video.name ?? ""}>{video.name} · {video.width}×{video.height}</span>
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => (video = null)}>Tanpa video</button>
      {:else}
        <div class="seg">
          {#each Object.entries(ASPECTS) as [key, a] (key)}
            <button
              class="seg-btn"
              class:active={aspect === key}
              type="button"
              onclick={() => (aspect = key as Aspect)}
            >{a.label}</button>
          {/each}
        </div>
      {/if}
      <button class="btn btn-sm" type="button" onclick={pickVideo}>Pilih Video…</button>
    </div>

    <div
      class="frame"
      class:portrait={frame.height > frame.width}
      style:aspect-ratio="{frame.width} / {frame.height}"
      bind:clientWidth={boxWidth}
    >
      {#if frame.src}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={frame.src} muted preload="auto" onloadedmetadata={seekToMiddle}></video>
      {/if}
      {#if scale > 0 && lines.length}
        <div
          class="caption"
          style:left="{metrics.marginH * scale}px"
          style:right="{metrics.marginH * scale}px"
          style:bottom="{metrics.marginV * scale}px"
          style:font-family="'{style.font_name}', Arial, sans-serif"
          style:font-size="{metrics.fontSize * scale}px"
          style:font-weight={style.bold ? 700 : 400}
          style:color={style.text_color}
        >
          {#each lines as line, i (i)}
            <span
              class="line"
              style:-webkit-text-stroke={style.background || metrics.outline === 0
                ? null
                : `${metrics.outline * 2 * scale}px ${style.outline_color}`}
              style:background={style.background ? rgba(style.outline_color, style.background_opacity) : null}
              style:padding={style.background ? `${boxPad * scale}px` : null}
            >{line}</span>
          {/each}
        </div>
      {/if}
    </div>
    {#if videoError}
      <p class="tip tip-bad">{videoError}</p>
    {/if}
    <p class="tip muted-2">
      ukuran mengikuti sisi pendek video, jadi tampil sama di video portrait maupun landscape.
      Klik <strong>Simpan Pengaturan</strong> supaya dipakai saat export.
    </p>
  </div>
</div>

<style>
  .editor {
    display: grid;
    gap: 16px;
  }
  @media (min-width: 760px) {
    .editor {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      align-items: start;
    }
  }
  .controls {
    display: grid;
    gap: 10px;
  }
  .opt {
    display: grid;
    gap: 6px;
  }
  .opt-label {
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted-2);
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .colors,
  .toggles {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
  }
  .color {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    cursor: pointer;
  }
  .color input {
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
  }
  .field {
    display: grid;
    gap: 6px;
  }
  .field label {
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted-2);
  }
  .field input {
    height: 32px;
    padding: 0 10px;
    background: var(--input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--foreground);
    font: inherit;
    font-size: 13px;
  }

  .preview-col {
    display: grid;
    gap: 8px;
    justify-items: center;
  }
  .preview-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    flex-wrap: wrap;
  }
  .vname {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg {
    display: inline-grid;
    grid-auto-flow: column;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    background: var(--surface-2);
    margin-right: auto;
  }
  .seg-btn {
    height: 26px;
    padding: 0 10px;
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

  .frame {
    position: relative;
    width: 100%;
    overflow: hidden;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background:
      radial-gradient(circle at 30% 25%, hsl(230 40% 45%), transparent 60%),
      radial-gradient(circle at 75% 70%, hsl(15 60% 45%), transparent 55%),
      hsl(230 20% 18%);
  }
  .frame.portrait {
    width: auto;
    height: min(520px, 70vh);
    max-width: 100%;
  }
  .frame video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .caption {
    position: absolute;
    text-align: center;
    line-height: 1.2;
    display: flex;
    flex-direction: column;
    align-items: center;
    pointer-events: none;
  }
  .line {
    paint-order: stroke fill;
    white-space: nowrap;
    -webkit-box-decoration-break: clone;
    box-decoration-break: clone;
  }

  .tip {
    margin: 0;
    font-size: 11px;
    line-height: 1.5;
    width: 100%;
  }
  .tip-bad {
    color: var(--neg);
  }
</style>
