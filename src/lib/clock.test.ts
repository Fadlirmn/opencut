import { describe, expect, it } from "vitest";
import { fmtClock, fmtDuration } from "./clock";

describe("fmtClock", () => {
  it("formats minutes and hours", () => {
    expect(fmtClock(0)).toBe("00:00");
    expect(fmtClock(75.4)).toBe("01:15");
    expect(fmtClock(3725)).toBe("1:02:05");
  });
});

describe("fmtDuration", () => {
  it("picks a readable unit", () => {
    expect(fmtDuration(8)).toBe("8 dtk");
    expect(fmtDuration(192)).toBe("3 mnt 12 dtk");
    expect(fmtDuration(180)).toBe("3 mnt");
    expect(fmtDuration(3840)).toBe("1 j 4 mnt");
  });
});
