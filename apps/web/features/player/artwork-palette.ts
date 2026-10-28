/**
 * Colour taken from album artwork for Now Playing and album heroes. The
 * dominant *vivid* hue wins (a grey sleeve with one red stripe gives red),
 * then it is tamed: lightness clamped so ivory text stays readable on the
 * glow, saturation capped so it never shouts louder than the brand copper.
 */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export interface ArtworkPalette {
  /** The artwork's colour, softened for glows and tints. */
  glow: Rgb;
  /** A very dark version for the backdrop base. */
  deep: Rgb;
  /** Whether the artwork had any real colour (not black and white). */
  chromatic: boolean;
}

/** Brand copper: the fallback when artwork can't be read. */
export const fallbackPalette: ArtworkPalette = {
  glow: { r: 217, g: 139, b: 95 },
  deep: { r: 22, g: 14, b: 10 },
  chromatic: false,
};

export function rgbToHsl({ r, g, b }: Rgb): [number, number, number] {
  const red = r / 255;
  const green = g / 255;
  const blue = b / 255;
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const lightness = (max + min) / 2;
  if (max === min) return [0, 0, lightness];
  const delta = max - min;
  const saturation =
    lightness > 0.5 ? delta / (2 - max - min) : delta / (max + min);
  let hue =
    max === red
      ? (green - blue) / delta + (green < blue ? 6 : 0)
      : max === green
        ? (blue - red) / delta + 2
        : (red - green) / delta + 4;
  hue *= 60;
  return [hue, saturation, lightness];
}

export function hslToRgb(hue: number, saturation: number, lightness: number) {
  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const section = (((hue % 360) + 360) % 360) / 60;
  const x = chroma * (1 - Math.abs((section % 2) - 1));
  const [red, green, blue] =
    section < 1
      ? [chroma, x, 0]
      : section < 2
        ? [x, chroma, 0]
        : section < 3
          ? [0, chroma, x]
          : section < 4
            ? [0, x, chroma]
            : section < 5
              ? [x, 0, chroma]
              : [chroma, 0, x];
  const match = lightness - chroma / 2;
  return {
    r: Math.round((red + match) * 255),
    g: Math.round((green + match) * 255),
    b: Math.round((blue + match) * 255),
  };
}

/**
 * Picks the palette from RGBA pixel data (any size; Now Playing samples a
 * 32×32 thumbnail). Pixels vote for one of 24 hue buckets weighted by
 * saturation and by being neither near-black nor near-white.
 */
export function paletteFromPixels(data: ArrayLike<number>): ArtworkPalette {
  const buckets = Array.from({ length: 24 }, () => ({
    weight: 0,
    r: 0,
    g: 0,
    b: 0,
  }));
  let total = { r: 0, g: 0, b: 0, count: 0 };
  for (let index = 0; index + 3 < data.length; index += 4) {
    const alpha = data[index + 3]! / 255;
    if (alpha < 0.5) continue;
    const pixel = { r: data[index]!, g: data[index + 1]!, b: data[index + 2]! };
    total = {
      r: total.r + pixel.r,
      g: total.g + pixel.g,
      b: total.b + pixel.b,
      count: total.count + 1,
    };
    const [hue, saturation, lightness] = rgbToHsl(pixel);
    const tone = 1 - Math.abs(lightness - 0.5) * 2; // 1 at mid-grey levels
    const weight = saturation * saturation * Math.max(0, tone);
    if (weight < 0.02) continue;
    const bucket = buckets[Math.floor(hue / 15) % 24]!;
    bucket.weight += weight;
    bucket.r += pixel.r * weight;
    bucket.g += pixel.g * weight;
    bucket.b += pixel.b * weight;
  }
  if (!total.count) return fallbackPalette;
  const best = buckets.reduce((winner, bucket) =>
    bucket.weight > winner.weight ? bucket : winner,
  );
  // Too little colour: a neutral, slightly warm glow from the average.
  const chromatic = best.weight / total.count > 0.03;
  const source: Rgb = chromatic
    ? {
        r: best.r / best.weight,
        g: best.g / best.weight,
        b: best.b / best.weight,
      }
    : {
        r: total.r / total.count,
        g: total.g / total.count,
        b: total.b / total.count,
      };
  const [hue, saturation] = rgbToHsl(source);
  const glow = chromatic
    ? hslToRgb(hue, Math.min(0.62, Math.max(0.28, saturation)), 0.56)
    : hslToRgb(30, 0.08, 0.6);
  const deep = chromatic
    ? hslToRgb(hue, Math.min(0.45, saturation), 0.07)
    : hslToRgb(30, 0.05, 0.05);
  return { glow, deep, chromatic };
}

export const rgbString = ({ r, g, b }: Rgb, alpha = 1) =>
  alpha >= 1
    ? `rgb(${Math.round(r)} ${Math.round(g)} ${Math.round(b)})`
    : `rgb(${Math.round(r)} ${Math.round(g)} ${Math.round(b)} / ${alpha})`;
