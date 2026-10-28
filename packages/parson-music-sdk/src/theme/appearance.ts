/**
 * A profile's appearance: theme preset, colour, shape, type, density, Home
 * rows, Now Playing layout and sidebar. Saved per profile on the server
 * (`/users/me/appearance`) and shared by the web app, the desktop app and
 * the phone app, which all read it through `normalizeAppearance` so a value
 * one client does not know never breaks another.
 */

import { argbFromHex, hexFromArgb } from "./color";

export const THEME_PRESETS = [
  "mujin-dark",
  "mujin-light",
  "material-you",
  "frosted-glass",
  "oled-black",
  "high-contrast",
] as const;
export type ThemePreset = (typeof THEME_PRESETS)[number];

export const COLOR_SCHEMES = ["system", "dark", "light"] as const;
export type ColorSchemePreference = (typeof COLOR_SCHEMES)[number];

export const DENSITIES = ["comfortable", "compact"] as const;
export type Density = (typeof DENSITIES)[number];

export const FONT_CHOICES = [
  "mujin",
  "inter",
  "fraunces",
  "atkinson",
  "plex",
  "roboto-flex",
] as const;
export type FontChoice = (typeof FONT_CHOICES)[number];

export const NOW_PLAYING_LAYOUTS = [
  "art-left",
  "art-centred",
  "immersive",
] as const;
export type NowPlayingLayout = (typeof NOW_PLAYING_LAYOUTS)[number];

export const SIDEBAR_MODES = ["expanded", "compact"] as const;
export type SidebarMode = (typeof SIDEBAR_MODES)[number];

export const MOTION_PREFERENCES = ["system", "reduced"] as const;
export type MotionPreference = (typeof MOTION_PREFERENCES)[number];

export const GLASS_MATERIALS = ["mica", "acrylic"] as const;
/** The Windows window material Frosted Glass asks for (macOS: vibrancy). */
export type GlassMaterial = (typeof GLASS_MATERIALS)[number];

export const TRANSPARENCY_PREFERENCES = ["system", "reduced"] as const;
export type TransparencyPreference = (typeof TRANSPARENCY_PREFERENCES)[number];

/** The rows Home can show, in their default order. */
export const HOME_ROWS = [
  "daily-picks",
  "recently-added",
  "recently-played",
  "new-for-you",
  "similar-artists",
  "best-years",
  "recommended-songs",
  "genres",
  "live-radio",
  "albums-you-might-like",
] as const;
export type HomeRowId = (typeof HOME_ROWS)[number];

export const HOME_ROW_LABELS: Record<HomeRowId, string> = {
  "daily-picks": "Daily Picks",
  "recently-added": "Recently added",
  "recently-played": "Recently played",
  "new-for-you": "New for you",
  "similar-artists": "Similar artists",
  "best-years": "In their prime",
  "recommended-songs": "Recommended songs",
  genres: "Genres",
  "live-radio": "Live radio",
  "albums-you-might-like": "Albums you might like",
};

export interface HomeRowSetting {
  id: HomeRowId;
  visible: boolean;
}

export interface Appearance {
  version: 1;
  preset: ThemePreset;
  /** Light or dark for presets that have both; `system` follows the OS. */
  color_scheme: ColorSchemePreference;
  /** `#rrggbb`, or null for the preset's own accent (Mujin Copper). */
  accent: string | null;
  /** Take the accent (and, for Material You, the scheme) from the art. */
  accent_from_artwork: boolean;
  /** Corner radius of cards and controls, in px at the default 8. */
  corner_radius: number;
  /** Text size, 0.85 to 1.3 (1 = default). */
  text_scale: number;
  density: Density;
  font: FontChoice;
  home_rows: HomeRowSetting[];
  now_playing_layout: NowPlayingLayout;
  sidebar: SidebarMode;
  motion: MotionPreference;
  transparency: TransparencyPreference;
  /** Frosted Glass in the Windows app: Mica (calm) or Acrylic (blurred). */
  glass_material: GlassMaterial;
  /** Android 12+: follow the system's wallpaper colours (Material You). */
  follow_system_colors: boolean;
}

export const DEFAULT_CORNER_RADIUS = 8;
export const CORNER_RADIUS_RANGE = { min: 0, max: 24 } as const;
export const TEXT_SCALE_RANGE = { min: 0.85, max: 1.3 } as const;

export const DEFAULT_APPEARANCE: Appearance = {
  version: 1,
  preset: "mujin-dark",
  color_scheme: "system",
  accent: null,
  accent_from_artwork: false,
  corner_radius: DEFAULT_CORNER_RADIUS,
  text_scale: 1,
  density: "comfortable",
  font: "mujin",
  home_rows: HOME_ROWS.map((id) => ({ id, visible: true })),
  now_playing_layout: "art-left",
  sidebar: "expanded",
  motion: "system",
  transparency: "system",
  glass_material: "mica",
  follow_system_colors: true,
};

export const PRESET_LABELS: Record<ThemePreset, string> = {
  "mujin-dark": "Mujin Dark",
  "mujin-light": "Mujin Light",
  "material-you": "Material You",
  "frosted-glass": "Frosted Glass",
  "oled-black": "OLED Black",
  "high-contrast": "High Contrast",
};

export const PRESET_DESCRIPTIONS: Record<ThemePreset, string> = {
  "mujin-dark":
    "Deep blacks, ivory type and one copper accent. The default gallery.",
  "mujin-light": "Ivory paper and ink, for daylight and bright rooms.",
  "material-you":
    "Material 3 colour from the playing album's art or your own colour, with rounder shapes and tonal surfaces.",
  "frosted-glass":
    "Translucent sidebar and panels over a blurred backdrop; native Mica, Acrylic or vibrancy in the desktop app.",
  "oled-black": "True black everywhere, so OLED pixels switch off.",
  "high-contrast":
    "Maximum contrast text, strong borders and focus rings, no transparency.",
};

export const FONT_LABELS: Record<FontChoice, string> = {
  mujin: "Mujin (Jost and Inter)",
  inter: "Inter",
  fraunces: "Fraunces and Inter",
  atkinson: "Atkinson Hyperlegible",
  plex: "IBM Plex Sans",
  "roboto-flex": "Roboto Flex",
};

export const NOW_PLAYING_LAYOUT_LABELS: Record<NowPlayingLayout, string> = {
  "art-left": "Art left",
  "art-centred": "Art centred",
  immersive: "Immersive",
};

/** Presets that come in a light and a dark form. */
export function presetHasLightAndDark(preset: ThemePreset): boolean {
  return (
    preset === "material-you" ||
    preset === "frosted-glass" ||
    preset === "high-contrast" ||
    preset === "mujin-dark" ||
    preset === "mujin-light"
  );
}

function pick<T extends string>(
  values: readonly T[],
  value: unknown,
  fallback: T,
): T {
  return typeof value === "string" &&
    (values as readonly string[]).includes(value)
    ? (value as T)
    : fallback;
}

function clampNumber(
  value: unknown,
  min: number,
  max: number,
  fallback: number,
) {
  const number = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.min(max, Math.max(min, number));
}

/** `#rrggbb` (lower case) or null. */
export function normalizeHexColor(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const argb = argbFromHex(value);
  return argb === null ? null : hexFromArgb(argb);
}

/**
 * Every known row once, in the saved order; rows a newer client added
 * appear at their default place, unknown ids are dropped.
 */
export function normalizeHomeRows(value: unknown): HomeRowSetting[] {
  const saved = Array.isArray(value) ? value : [];
  const rows: HomeRowSetting[] = [];
  const seen = new Set<HomeRowId>();
  for (const entry of saved) {
    const id =
      entry && typeof entry === "object"
        ? (entry as { id?: unknown }).id
        : undefined;
    if (typeof id !== "string") continue;
    if (!(HOME_ROWS as readonly string[]).includes(id)) continue;
    const rowId = id as HomeRowId;
    if (seen.has(rowId)) continue;
    seen.add(rowId);
    rows.push({
      id: rowId,
      visible: (entry as { visible?: unknown }).visible !== false,
    });
  }
  HOME_ROWS.forEach((id, defaultIndex) => {
    if (seen.has(id)) return;
    // After the closest earlier default row that is present, else first.
    let at = 0;
    for (let earlier = defaultIndex - 1; earlier >= 0; earlier -= 1) {
      const found = rows.findIndex((row) => row.id === HOME_ROWS[earlier]);
      if (found >= 0) {
        at = found + 1;
        break;
      }
    }
    rows.splice(at, 0, { id, visible: true });
    seen.add(id);
  });
  return rows;
}

/** A complete, valid appearance from anything (a saved value, an edit). */
export function normalizeAppearance(value: unknown): Appearance {
  const input =
    value && typeof value === "object"
      ? (value as Record<string, unknown>)
      : {};
  const base = DEFAULT_APPEARANCE;
  return {
    version: 1,
    preset: pick(THEME_PRESETS, input.preset, base.preset),
    color_scheme: pick(COLOR_SCHEMES, input.color_scheme, base.color_scheme),
    accent: normalizeHexColor(input.accent),
    accent_from_artwork: input.accent_from_artwork === true,
    corner_radius: Math.round(
      clampNumber(
        input.corner_radius,
        CORNER_RADIUS_RANGE.min,
        CORNER_RADIUS_RANGE.max,
        base.corner_radius,
      ),
    ),
    text_scale:
      Math.round(
        clampNumber(
          input.text_scale,
          TEXT_SCALE_RANGE.min,
          TEXT_SCALE_RANGE.max,
          base.text_scale,
        ) * 100,
      ) / 100,
    density: pick(DENSITIES, input.density, base.density),
    font: pick(FONT_CHOICES, input.font, base.font),
    home_rows: normalizeHomeRows(input.home_rows),
    now_playing_layout: pick(
      NOW_PLAYING_LAYOUTS,
      input.now_playing_layout,
      base.now_playing_layout,
    ),
    sidebar: pick(SIDEBAR_MODES, input.sidebar, base.sidebar),
    motion: pick(MOTION_PREFERENCES, input.motion, base.motion),
    transparency: pick(
      TRANSPARENCY_PREFERENCES,
      input.transparency,
      base.transparency,
    ),
    glass_material: pick(
      GLASS_MATERIALS,
      input.glass_material,
      base.glass_material,
    ),
    follow_system_colors: input.follow_system_colors !== false,
  };
}

/** The visible Home rows, in order. */
export function visibleHomeRows(appearance: Pick<Appearance, "home_rows">) {
  return normalizeHomeRows(appearance.home_rows)
    .filter((row) => row.visible)
    .map((row) => row.id);
}

/** Moves a Home row up (-1) or down (+1); unchanged at either end. */
export function moveHomeRow(
  rows: readonly HomeRowSetting[],
  id: HomeRowId,
  direction: -1 | 1,
): HomeRowSetting[] {
  const next = [...rows];
  const from = next.findIndex((row) => row.id === id);
  const to = from + direction;
  if (from < 0 || to < 0 || to >= next.length) return next;
  const [row] = next.splice(from, 1);
  next.splice(to, 0, row!);
  return next;
}

/** Moves a Home row to an index (drag and drop). */
export function placeHomeRow(
  rows: readonly HomeRowSetting[],
  id: HomeRowId,
  index: number,
): HomeRowSetting[] {
  const next = [...rows];
  const from = next.findIndex((row) => row.id === id);
  if (from < 0) return next;
  const [row] = next.splice(from, 1);
  next.splice(Math.max(0, Math.min(next.length, index)), 0, row!);
  return next;
}

/** Whether the appearance is exactly the default (for "Reset" buttons). */
export function isDefaultAppearance(appearance: Appearance): boolean {
  return (
    JSON.stringify(normalizeAppearance(appearance)) ===
    JSON.stringify(DEFAULT_APPEARANCE)
  );
}
