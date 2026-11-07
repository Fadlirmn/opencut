/// Mirror of the caption layout in `src-tauri/src/subtitle.rs`, used by the
/// Settings preview so what you see there matches what gets burned in.
/// Keep the numbers in sync with `metrics()` / `wrap_text()` on the Rust side.

export type SubtitleStyle = {
  font_name: string;
  /// Glyph size, % of the frame's short side.
  size_pct: number;
  /// Bottom edge → bottom of caption, % of frame height.
  position_pct: number;
  text_color: string;
  outline_color: string;
  /// Outline thickness, 1/1000ths of the short side.
  outline: number;
  bold: boolean;
  uppercase: boolean;
  background: boolean;
  background_opacity: number;
  /// 0 = automatic from frame width and font size.
  max_chars_per_line: number;
  /// Words on screen at once; 0 = whole transcript segment.
  words_per_caption: number;
};

export const DEFAULT_SUBTITLE_STYLE: SubtitleStyle = {
  font_name: "Arial",
  size_pct: 11,
  position_pct: 25,
  text_color: "#FFFFFF",
  outline_color: "#000000",
  outline: 5,
  bold: true,
  uppercase: false,
  background: false,
  background_opacity: 60,
  max_chars_per_line: 0,
  words_per_caption: 1,
};

export type SubtitleMetrics = {
  fontSize: number;
  outline: number;
  marginV: number;
  marginH: number;
  maxChars: number;
};

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function subtitleMetrics(width: number, height: number, style: SubtitleStyle): SubtitleMetrics {
  const w = Math.max(2, width);
  const h = Math.max(2, height);
  const short = Math.min(w, h);
  const sizePct = Number.isFinite(style.size_pct) ? clamp(style.size_pct, 2, 30) : 11;
  const fontSize = Math.max(8, Math.round((short * sizePct) / 100));
  const outline = Math.round(((short * clamp(style.outline, 0, 30)) / 1000) * 10) / 10;
  const positionPct = Number.isFinite(style.position_pct) ? clamp(style.position_pct, 0, 90) : 25;
  const marginV = Math.round((h * positionPct) / 100);
  const marginH = Math.max(12, Math.round(w * 0.05));
  const charEm = style.bold ? 0.55 : 0.5;
  const maxChars =
    style.max_chars_per_line > 0
      ? style.max_chars_per_line
      : clamp(Math.floor((w - 2 * marginH) / (fontSize * charEm)), 8, 60);
  return { fontSize, outline, marginV, marginH, maxChars };
}

/// Port of `wrap_text`: at most two lines, the second balanced against the
/// first instead of leaving an orphan word.
export function wrapCaption(text: string, maxChars: number): string[] {
  const max = Math.max(8, maxChars);
  const words = text.split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const joined = words.join(" ");
  if ([...joined].length <= max) return [joined];

  const lines: string[] = [];
  let current = "";
  for (const word of words) {
    const len = current ? [...current].length + 1 + [...word].length : [...word].length;
    if (len > max && current) {
      lines.push(current);
      current = word;
    } else {
      current = current ? `${current} ${word}` : word;
    }
  }
  if (current) lines.push(current);
  if (lines.length <= 2) return lines;

  const target = Math.floor([...joined].length / 2);
  let length = 0;
  let splitAt = 0;
  let best = Infinity;
  words.forEach((word, i) => {
    const candidate = length + [...word].length + (i > 0 ? 1 : 0);
    if (Math.abs(candidate - target) < best) {
      best = Math.abs(candidate - target);
      splitAt = i + 1;
    }
    length = candidate;
  });
  splitAt = clamp(splitAt, 1, words.length - 1);
  return [words.slice(0, splitAt).join(" "), words.slice(splitAt).join(" ")];
}

/// Mirror of `chunk_segments`: the caption texts one segment becomes, each
/// with its share of the segment's time (0–1), weighted by characters + 1
/// per word.
export function chunkCaption(text: string, words: number): { text: string; share: number }[] {
  const all = text.split(/\s+/).filter(Boolean);
  if (all.length === 0) return [];
  if (words <= 0) return [{ text: all.join(" "), share: 1 }];
  const groups: string[][] = [];
  for (let i = 0; i < all.length; i += words) groups.push(all.slice(i, i + words));
  const weight = (g: string[]) => g.reduce((a, w) => a + [...w].length + 1, 0);
  const total = groups.reduce((a, g) => a + weight(g), 0);
  return groups.map((g) => ({ text: g.join(" "), share: weight(g) / total }));
}

/// `#RRGGBB` + opacity (0–100) → CSS rgba().
export function rgba(hex: string, opacityPct = 100): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  const n = m ? parseInt(m[1], 16) : 0;
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${clamp(opacityPct, 0, 100) / 100})`;
}
