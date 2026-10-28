import { describe, expect, test } from "bun:test";
import { brandSignalColor } from "./brand-signal-light";

describe("the logo's signal dot", () => {
  test("keeps its default copper when nothing is playing", () => {
    expect(brandSignalColor(null)).toBeNull();
    expect(brandSignalColor(undefined)).toBeNull();
  });

  // The theme variable carries the light in the active theme's tone; the
  // brand hex is its fallback.
  test("follows the signal-path light", () => {
    expect(brandSignalColor({ quality: "lossless", clipping: false })).toBe(
      "var(--color-quality-lossless, #D98B5F)",
    );
    expect(brandSignalColor({ quality: "enhanced", clipping: false })).toBe(
      "var(--color-quality-enhanced, #8FC3E0)",
    );
    expect(brandSignalColor({ quality: "high_quality", clipping: false })).toBe(
      "var(--color-quality-high, #A3C98B)",
    );
    expect(brandSignalColor({ quality: "low_quality", clipping: false })).toBe(
      "var(--color-quality-low, #7E7A74)",
    );
    expect(brandSignalColor({ quality: "lossless", clipping: true })).toBe(
      "var(--color-quality-clipping, #F2473F)",
    );
  });
});
