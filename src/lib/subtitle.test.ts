import { describe, expect, test } from "vitest";
import { DEFAULT_SUBTITLE_STYLE, rgba, subtitleMetrics, wrapCaption } from "./subtitle";

describe("subtitleMetrics", () => {
  test("matches the Rust renderer for the default style", () => {
    // Same numbers asserted in subtitle.rs: 7% of a 1080 short side.
    expect(subtitleMetrics(1080, 1920, DEFAULT_SUBTITLE_STYLE).fontSize).toBe(76);
    expect(subtitleMetrics(1920, 1080, DEFAULT_SUBTITLE_STYLE).fontSize).toBe(76);
  });

  test("position sets the bottom margin", () => {
    const m = subtitleMetrics(1080, 1920, { ...DEFAULT_SUBTITLE_STYLE, position_pct: 10 });
    expect(m.marginV).toBe(192);
  });

  test("explicit line length wins over auto", () => {
    const m = subtitleMetrics(1080, 1920, { ...DEFAULT_SUBTITLE_STYLE, max_chars_per_line: 30 });
    expect(m.maxChars).toBe(30);
  });
});

describe("wrapCaption", () => {
  test("short text stays on one line", () => {
    expect(wrapCaption("halo dunia", 20)).toEqual(["halo dunia"]);
  });

  test("long text becomes two balanced lines keeping every word", () => {
    const text = "satu dua tiga empat lima enam tujuh delapan sembilan sepuluh sebelas dua belas";
    const lines = wrapCaption(text, 24);
    expect(lines).toHaveLength(2);
    expect(lines.join(" ")).toBe(text);
  });
});

test("rgba converts hex and opacity", () => {
  expect(rgba("#FF8000", 50)).toBe("rgba(255, 128, 0, 0.5)");
  expect(rgba("nope")).toBe("rgba(0, 0, 0, 1)");
});
