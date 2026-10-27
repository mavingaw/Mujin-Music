/**
 * Display mode on a tablet or phone: the screen follows one zone (or what
 * plays) full screen, like a TV display. These helpers size the layout and
 * name where the screen keeps its display key.
 */

/** Where this device keeps the display key it made for a zone. */
export function displayKeyStorageName(zoneId: string | null): string {
  const slot = zoneId ? zoneId.replace(/[^A-Za-z0-9._-]/g, "_") : "follow";
  return `mujin.display.${slot}`.slice(0, 120);
}

export type DisplayMetrics = {
  landscape: boolean;
  /** Side gutter. */
  gutter: number;
  cover: number;
  coverInLyrics: number;
  title: number;
  titleInLyrics: number;
  artist: number;
  lyric: number;
  clock: number;
};

/** Type and artwork sizes for a screen `width` × `height` points. */
export function displayMetrics(width: number, height: number): DisplayMetrics {
  const landscape = width > height;
  const short = Math.max(320, Math.min(width, height));
  const clamp = (value: number, min: number, max: number) =>
    Math.round(Math.min(max, Math.max(min, value)));
  return {
    landscape,
    gutter: clamp(width * 0.05, 18, 72),
    cover: landscape
      ? clamp(height * 0.34, 110, 360)
      : clamp(width * 0.3, 96, 220),
    coverInLyrics: landscape
      ? clamp(height * 0.16, 64, 180)
      : clamp(width * 0.18, 56, 120),
    title: clamp(short * (landscape ? 0.085 : 0.09), 28, 96),
    titleInLyrics: clamp(short * 0.055, 22, 56),
    artist: clamp(short * 0.042, 16, 40),
    lyric: clamp(short * (landscape ? 0.07 : 0.075), 24, 76),
    clock: clamp(short * 0.05, 20, 48),
  };
}

/** The lines shown around the sung one: two behind, four ahead. */
export function visibleLyricRange(
  length: number,
  active: number,
): { start: number; end: number } {
  if (length <= 0) return { start: 0, end: 0 };
  const anchor = Math.max(0, active);
  return {
    start: Math.max(0, anchor - 2),
    end: Math.min(length, anchor + 5),
  };
}
