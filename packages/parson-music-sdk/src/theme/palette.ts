/**
 * Tonal palettes (one hue and chroma, every tone 0-100) and contrast-aware
 * tone adjustment: the building blocks of every generated theme.
 */

import { type Argb, contrastRatio } from "./color";
import { argbFromHct, hctFromArgb } from "./hct";

export class TonalPalette {
  private readonly cache = new Map<number, Argb>();

  constructor(
    readonly hue: number,
    readonly chroma: number,
  ) {}

  static fromArgb(argb: Argb): TonalPalette {
    const hct = hctFromArgb(argb);
    return new TonalPalette(hct.hue, hct.chroma);
  }

  tone(tone: number): Argb {
    const key = Math.round(Math.max(0, Math.min(100, tone)) * 10) / 10;
    let argb = this.cache.get(key);
    if (argb === undefined) {
      argb = argbFromHct(this.hue, this.chroma, key);
      this.cache.set(key, argb);
    }
    return argb;
  }
}

/**
 * The colour with `argb`'s hue and chroma at the nearest tone that reaches
 * `ratio` contrast against `background`. Tones move away from the
 * background (lighter on dark, darker on light); if that end of the scale
 * cannot reach the ratio, the other direction is tried. Returns the colour
 * unchanged when it already passes.
 */
export function ensureContrast(
  argb: Argb,
  background: Argb,
  ratio: number,
): Argb {
  if (contrastRatio(argb, background) >= ratio) return argb;
  const hct = hctFromArgb(argb);
  const backgroundTone = hctFromArgb(background).tone;
  const preferLighter = backgroundTone < 50;
  const search = (lighter: boolean): Argb | null => {
    for (let step = 1; step <= 100; step += 1) {
      const tone = hct.tone + (lighter ? step : -step);
      if (tone < 0 || tone > 100) return null;
      const candidate = argbFromHct(hct.hue, hct.chroma, tone);
      if (contrastRatio(candidate, background) >= ratio) return candidate;
    }
    return null;
  };
  return (
    search(preferLighter) ??
    search(!preferLighter) ??
    (preferLighter ? 0xffffffff : 0xff000000)
  );
}

/** Whichever of `light` and `dark` reads better on `background`. */
export function bestOn(background: Argb, light: Argb, dark: Argb): Argb {
  return contrastRatio(light, background) >= contrastRatio(dark, background)
    ? light
    : dark;
}
