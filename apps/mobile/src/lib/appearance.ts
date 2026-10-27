import {
  type Appearance,
  type ResolvedTheme,
  type ThemeEnvironment,
  argbFromHex,
  DEFAULT_APPEARANCE,
  normalizeAppearance,
  resolveTheme,
  rgba,
} from "@parson/music-sdk";

/**
 * The phone app's side of the theme engine. It shares the SDK's presets,
 * HCT palettes and WCAG checks with the web and desktop apps, and maps the
 * resolved tokens onto the palette its screens already style with.
 */

export const APPEARANCE_STORE_KEY = "mujin.appearance.v1";

/** What the phone keeps between launches (SecureStore, read synchronously). */
export interface StoredAppearance {
  appearance: Appearance;
  revision: number;
  /** The last playing cover's source colour, for colour from artwork. */
  artworkColor: number | null;
}

export function parseStoredAppearance(raw: string | null): StoredAppearance {
  try {
    const value = raw ? (JSON.parse(raw) as Partial<StoredAppearance>) : null;
    return {
      appearance: normalizeAppearance(value?.appearance),
      revision: Number(value?.revision) || 0,
      artworkColor:
        typeof value?.artworkColor === "number" ? value.artworkColor : null,
    };
  } catch {
    return { appearance: DEFAULT_APPEARANCE, revision: 0, artworkColor: null };
  }
}

export function serializeStoredAppearance(stored: StoredAppearance): string {
  return JSON.stringify(stored);
}

/** Android's signed ARGB integers as the SDK's unsigned ARGB. */
export function argbFromAndroidColor(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return (value | 0xff000000) >>> 0;
}

export interface MobilePalette {
  mode: "dark" | "light";
  background: string;
  elevated: string;
  elevatedStrong: string;
  border: string;
  borderStrong: string;
  text: string;
  secondary: string;
  muted: string;
  accent: string;
  accentSoft: string;
  accentDeep: string;
  accentText: string;
  accentContainer: string;
  onAccentContainer: string;
  qualityHigh: string;
  qualityLow: string;
  danger: string;
  success: string;
  tertiary: string;
  surface: string;
  /** The deepest surface (Now Playing, OLED black). */
  deep: string;
  /** The highest surface step (pressed cards, tracks). */
  surfaceStrong: string;
  /** The signal path's "enhanced" blue and the brass status colour. */
  enhanced: string;
  brass: string;
  /** Text colour at low alpha: hairlines, washes, pressed states. */
  wash: string;
  /** A lighter wash for selected rows and chips. */
  washStrong: string;
}

/** The theme's tokens as the phone palette. */
export function mobilePalette(theme: ResolvedTheme): MobilePalette {
  const c = theme.colors;
  const ink = argbFromHex(c.text) ?? 0xfff2eee6;
  const high = theme.contrast === "high";
  return {
    mode: theme.mode,
    background: c.canvas,
    elevated: c.raised,
    elevatedStrong: c.surface2,
    border: high ? c.border : rgba(ink, theme.mode === "dark" ? 0.1 : 0.12),
    borderStrong: high ? c.borderStrong : rgba(ink, 0.18),
    text: c.text,
    secondary: c.textSecondary,
    muted: c.textDisabled,
    accent: c.accent,
    accentSoft: c.accentHover,
    accentDeep: c.accentPressed,
    accentText: c.onAccent,
    accentContainer: c.accentContainer,
    onAccentContainer: c.onAccentContainer,
    qualityHigh: c.qualityHigh,
    qualityLow: c.qualityLow,
    danger: c.danger,
    success: c.qualityHigh,
    tertiary: c.textTertiary,
    surface: c.surface1,
    deep: c.deep,
    surfaceStrong: c.surface3,
    enhanced: c.qualityEnhanced,
    brass: theme.statusRamps?.brass[4] ?? "#daab6c",
    wash: rgba(ink, 0.08),
    washStrong: rgba(ink, 0.14),
  };
}

/** Any theme colour at `alpha` (`#rrggbb` in, `rgba(...)` out). */
export function withAlpha(hex: string, alpha: number) {
  return rgba(argbFromHex(hex) ?? 0xff000000, alpha);
}

/** Text colour at `alpha`, for hand-made overlays. */
export function tintOf(palette: Pick<MobilePalette, "text">, alpha: number) {
  return rgba(argbFromHex(palette.text) ?? 0xfff2eee6, alpha);
}

/** Card and sheet radius on the phone (16 at the default 8px setting). */
export function mobileRadius(theme: ResolvedTheme): number {
  return Math.round(16 * theme.shape.scale);
}

/** Resolves the stored appearance for the phone's surroundings. */
export function resolveMobileTheme(
  stored: StoredAppearance,
  environment: Omit<ThemeEnvironment, "artworkColor">,
): ResolvedTheme {
  return resolveTheme(stored.appearance, {
    ...environment,
    artworkColor: stored.artworkColor,
  });
}
