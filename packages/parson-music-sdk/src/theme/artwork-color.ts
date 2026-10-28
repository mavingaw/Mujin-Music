/**
 * The source colour of an album cover for Material You: the colour a
 * person would name for the cover, chosen the way Material's "Score" does
 * it. Pixels are grouped (5 bits per channel), measured in HCT, and each
 * hue is rated by how much of the cover sits within 15 degrees of it, with
 * a bonus for chroma, so a small vivid detail can win over a large grey
 * field but not over a large coloured one.
 */

import { type Argb, argbFromRgb } from "./color";
import { hctFromArgb } from "./hct";

/** Mujin Copper: the answer for a cover with no usable colour. */
export const FALLBACK_SOURCE = 0xffd98b5f;

export interface ScoredColor {
  argb: Argb;
  hue: number;
  chroma: number;
  score: number;
}

/**
 * Ranked source colours from RGBA pixel data (a canvas `ImageData.data`,
 * or the same layout from a native decoder). `maxColors` caps the result.
 */
export function scoreArtworkColors(
  rgba: ArrayLike<number>,
  maxColors = 4,
): ScoredColor[] {
  const populations = new Map<number, number>();
  for (let index = 0; index + 3 < rgba.length; index += 4) {
    if ((rgba[index + 3] ?? 0) < 250) continue;
    const red = (rgba[index] ?? 0) & 0xf8;
    const green = (rgba[index + 1] ?? 0) & 0xf8;
    const blue = (rgba[index + 2] ?? 0) & 0xf8;
    const key = argbFromRgb(red | 4, green | 4, blue | 4);
    populations.set(key, (populations.get(key) ?? 0) + 1);
  }
  const total = [...populations.values()].reduce(
    (sum, count) => sum + count,
    0,
  );
  if (!total) return [];
  const hueProportion = new Array<number>(360).fill(0);
  const colors: {
    argb: Argb;
    hue: number;
    chroma: number;
    tone: number;
    count: number;
  }[] = [];
  for (const [argb, count] of populations) {
    const hct = hctFromArgb(argb);
    const hue = Math.floor(hct.hue) % 360;
    hueProportion[hue]! += count / total;
    colors.push({
      argb,
      hue: hct.hue,
      chroma: hct.chroma,
      tone: hct.tone,
      count,
    });
  }
  const excited = hueProportion.map((_, hue) => {
    let sum = 0;
    for (let offset = -15; offset <= 15; offset += 1) {
      sum += hueProportion[(hue + offset + 360) % 360]!;
    }
    return sum;
  });
  // The most chromatic, then most common, colour for each hue degree.
  const best = new Map<number, (typeof colors)[number]>();
  for (const color of colors) {
    if (color.chroma < 5 || color.tone < 12 || color.tone > 92) continue;
    const hue = Math.floor(color.hue) % 360;
    const current = best.get(hue);
    if (
      !current ||
      color.count * Math.max(color.chroma, 1) >
        current.count * Math.max(current.chroma, 1)
    ) {
      best.set(hue, color);
    }
  }
  const scored: ScoredColor[] = [];
  for (const [hue, color] of best) {
    const proportion = excited[hue]!;
    if (proportion <= 0.01) continue;
    const chromaScore = (color.chroma - 48) * (color.chroma < 48 ? 0.1 : 0.3);
    scored.push({
      argb: color.argb,
      hue: color.hue,
      chroma: color.chroma,
      score: proportion * 100 * 0.7 + chromaScore,
    });
  }
  scored.sort((a, b) => b.score - a.score);
  const chosen: ScoredColor[] = [];
  for (const candidate of scored) {
    const distinct = chosen.every((color) => {
      const difference = Math.abs(color.hue - candidate.hue) % 360;
      return Math.min(difference, 360 - difference) >= 15;
    });
    if (distinct) chosen.push(candidate);
    if (chosen.length >= maxColors) break;
  }
  return chosen;
}

/** The single best source colour, or Mujin Copper for a colourless cover. */
export function sourceColorFromPixels(rgba: ArrayLike<number>): Argb {
  return scoreArtworkColors(rgba, 1)[0]?.argb ?? FALLBACK_SOURCE;
}
