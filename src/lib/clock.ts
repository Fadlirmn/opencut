/// `mm:ss`, or `h:mm:ss` past an hour. Mirrors `progress::fmt_clock` in Rust.
export function fmtClock(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(r).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/// Short human duration for elapsed/ETA labels: "8 dtk", "3 mnt 12 dtk", "1 j 4 mnt".
export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} dtk`;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  if (h > 0) return `${h} j ${m} mnt`;
  return r ? `${m} mnt ${r} dtk` : `${m} mnt`;
}
