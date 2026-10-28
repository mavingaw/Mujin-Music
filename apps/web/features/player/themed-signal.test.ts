import { expect, test } from "bun:test";
import { signalLightColor, signalQualityColors } from "./themed-signal";

test("signal colours follow the theme, with the brand hex as fallback", () => {
  expect(signalQualityColors.lossless).toBe(
    "var(--color-quality-lossless, #D98B5F)",
  );
  expect(signalLightColor({ quality: "high_quality", clipping: false })).toBe(
    "var(--color-quality-high, #A3C98B)",
  );
  expect(signalLightColor({ quality: "lossless", clipping: true })).toContain(
    "--color-quality-clipping",
  );
});
