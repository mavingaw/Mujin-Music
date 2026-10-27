import { expect, test } from "bun:test";
import {
  fallbackPalette,
  hslToRgb,
  paletteFromPixels,
  rgbString,
  rgbToHsl,
} from "./artwork-palette";

function pixels(...colours: [number, number, number, number][]) {
  return colours.flatMap(([r, g, b, count]) =>
    Array.from({ length: count }, () => [r, g, b, 255]).flat(),
  );
}

test("hsl conversions round-trip", () => {
  for (const colour of [
    { r: 217, g: 139, b: 95 },
    { r: 20, g: 40, b: 200 },
    { r: 128, g: 128, b: 128 },
  ]) {
    const [h, s, l] = rgbToHsl(colour);
    const back = hslToRgb(h, s, l);
    expect(Math.abs(back.r - colour.r)).toBeLessThanOrEqual(1);
    expect(Math.abs(back.g - colour.g)).toBeLessThanOrEqual(1);
    expect(Math.abs(back.b - colour.b)).toBeLessThanOrEqual(1);
  }
});

test("a vivid accent beats a large neutral area", () => {
  const palette = paletteFromPixels(
    pixels([120, 120, 120, 800], [200, 30, 30, 120], [10, 10, 10, 100]),
  );
  expect(palette.chromatic).toBe(true);
  const [hue, , lightness] = rgbToHsl(palette.glow);
  expect(hue < 15 || hue > 345).toBe(true);
  expect(lightness).toBeCloseTo(0.56, 1);
  expect(rgbToHsl(palette.deep)[2]).toBeLessThan(0.1);
});

test("black and white artwork gives a neutral warm glow", () => {
  const palette = paletteFromPixels(
    pixels([0, 0, 0, 500], [250, 250, 250, 500], [128, 128, 128, 50]),
  );
  expect(palette.chromatic).toBe(false);
  const [, saturation] = rgbToHsl(palette.glow);
  expect(saturation).toBeLessThan(0.15);
});

test("empty or transparent data falls back to copper", () => {
  expect(paletteFromPixels([])).toEqual(fallbackPalette);
  expect(paletteFromPixels([255, 0, 0, 0])).toEqual(fallbackPalette);
});

test("rgb strings use modern syntax", () => {
  expect(rgbString({ r: 1, g: 2.4, b: 3 })).toBe("rgb(1 2 3)");
  expect(rgbString({ r: 1, g: 2, b: 3 }, 0.5)).toBe("rgb(1 2 3 / 0.5)");
});
