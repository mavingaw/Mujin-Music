import { describe, expect, test } from "bun:test";
import { brandSignalColor } from "./brand-signal-light";

describe("the logo's signal dot", () => {
  test("keeps its default copper when nothing is playing", () => {
    expect(brandSignalColor(null)).toBeNull();
    expect(brandSignalColor(undefined)).toBeNull();
  });

  test("follows the signal-path light", () => {
    expect(brandSignalColor({ quality: "lossless", clipping: false })).toBe(
      "#D98B5F",
    );
    expect(brandSignalColor({ quality: "enhanced", clipping: false })).toBe(
      "#8FC3E0",
    );
    expect(brandSignalColor({ quality: "high_quality", clipping: false })).toBe(
      "#A3C98B",
    );
    expect(brandSignalColor({ quality: "low_quality", clipping: false })).toBe(
      "#7E7A74",
    );
    expect(brandSignalColor({ quality: "lossless", clipping: true })).toBe(
      "#F2473F",
    );
  });
});
