import {
  type Appearance,
  type ResolvedTheme,
  type ThemeEnvironment,
  normalizeAppearance,
  resolveTheme,
  themeAttributes,
  themeCss,
} from "@parson/music-sdk";

/**
 * The last applied theme, kept in localStorage so the next page load paints
 * in the right colours before React starts (no dark flash for a light
 * theme). Both the dark and light forms are stored, so a theme that
 * follows the system is right even if the system switched overnight.
 */
export { THEME_CACHE_KEY, THEME_STYLE_ID } from "./theme-boot";

export interface ThemeCacheEntry {
  css: string;
  attrs: Record<string, string | null>;
  themeColor: string;
}

export interface ThemeCache {
  v: 1;
  userId: string | null;
  appearance: Appearance;
  revision: number;
  dark: ThemeCacheEntry;
  light: ThemeCacheEntry;
}

export function cacheEntry(theme: ResolvedTheme): ThemeCacheEntry {
  return {
    css: themeCss(theme),
    attrs: themeAttributes(theme),
    themeColor: theme.colors.canvas,
  };
}

export function buildThemeCache(
  appearance: Appearance,
  revision: number,
  environment: Omit<ThemeEnvironment, "systemDark">,
  userId: string | null,
): ThemeCache {
  return {
    v: 1,
    userId,
    appearance,
    revision,
    dark: cacheEntry(
      resolveTheme(appearance, { ...environment, systemDark: true }),
    ),
    light: cacheEntry(
      resolveTheme(appearance, { ...environment, systemDark: false }),
    ),
  };
}

/** A cached theme, or null when it is missing or from an older version. */
export function readThemeCache(raw: string | null): ThemeCache | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<ThemeCache>;
    if (value?.v !== 1 || !value.dark?.css || !value.light?.css) return null;
    return {
      v: 1,
      userId: typeof value.userId === "string" ? value.userId : null,
      appearance: normalizeAppearance(value.appearance),
      revision: Number(value.revision) || 0,
      dark: value.dark,
      light: value.light,
    };
  } catch {
    return null;
  }
}
