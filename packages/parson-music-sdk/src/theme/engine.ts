/**
 * The theme engine: an appearance plus its surroundings (system light or
 * dark, the playing album's colour, reduced transparency) resolved into
 * every colour token the apps draw with. Generated palettes are checked
 * against WCAG 2.x and their tones moved until text reaches AA (AAA-level
 * targets for High Contrast), so a colour picked by a person or taken from
 * a cover can never make the app unreadable.
 */

import { type Argb, argbFromHex, contrastRatio, hexFromArgb } from "./color";
import { hctFromArgb } from "./hct";
import { bestOn, ensureContrast, TonalPalette } from "./palette";
import {
  type Appearance,
  type Density,
  type FontChoice,
  type NowPlayingLayout,
  type SidebarMode,
  type ThemePreset,
  DEFAULT_CORNER_RADIUS,
  normalizeAppearance,
  PRESET_LABELS,
} from "./appearance";

export type ThemeMode = "dark" | "light";
export type NativeMaterial = "mica" | "acrylic" | "vibrancy";

export interface ThemeEnvironment {
  /** `prefers-color-scheme: dark` (or the phone's dark mode). */
  systemDark: boolean;
  /** The playing album's source colour, when known. */
  artworkColor?: Argb | null;
  /** Android 12+ wallpaper seed colour, when the platform exposes it. */
  systemAccent?: Argb | null;
  /** `prefers-reduced-transparency: reduce`. */
  reducedTransparency?: boolean;
  /** `prefers-reduced-motion: reduce`. */
  reducedMotion?: boolean;
  /** Window materials the desktop shell can show behind the page. */
  nativeMaterials?: readonly NativeMaterial[];
}

/** Semantic colours, all `#rrggbb`. */
export interface ThemeColors {
  canvas: string;
  deep: string;
  raised: string;
  surface1: string;
  surface2: string;
  surface3: string;
  border: string;
  borderStrong: string;
  text: string;
  textSecondary: string;
  textTertiary: string;
  textDisabled: string;
  accent: string;
  accentHover: string;
  accentPressed: string;
  onAccent: string;
  accentContainer: string;
  onAccentContainer: string;
  danger: string;
  qualityLossless: string;
  qualityEnhanced: string;
  qualityHigh: string;
  qualityLow: string;
  qualityClipping: string;
}

export const STATUS_FAMILIES = [
  "enhanced",
  "high",
  "brass",
  "clipping",
] as const;
export type StatusFamily = (typeof STATUS_FAMILIES)[number];

/** One colour family as Tailwind's 50..950 steps (11 values). */
export type Ramp = string[];

export interface ContrastCheck {
  name: string;
  foreground: string;
  background: string;
  ratio: number;
  minimum: number;
  /** The tone was moved to reach the minimum. */
  adjusted: boolean;
}

export type AccentSource = "brand" | "custom" | "artwork" | "system";

export interface ResolvedTheme {
  preset: ThemePreset;
  label: string;
  mode: ThemeMode;
  contrast: "standard" | "high";
  colors: ThemeColors;
  /** Neutral steps for Tailwind's zinc (null: the stylesheet's brand values). */
  neutralRamp: Ramp | null;
  /** Accent steps (null: the stylesheet's copper). */
  accentRamp: Ramp | null;
  /** Status families re-toned for this mode (null: the stylesheet's). */
  statusRamps: Record<StatusFamily, Ramp> | null;
  shape: { scale: number; material: boolean };
  glass: {
    enabled: boolean;
    native: NativeMaterial | null;
    /** Opacity of glass panels over the backdrop, 0..1. */
    panelOpacity: number;
    blurPx: number;
    noise: boolean;
  };
  type: {
    scale: number;
    font: FontChoice;
    display: string;
    body: string;
    mono: string;
    material: boolean;
  };
  density: Density;
  motion: "full" | "reduced";
  nowPlayingLayout: NowPlayingLayout;
  sidebar: SidebarMode;
  accentSource: AccentSource;
  checks: ContrastCheck[];
}

export const MUJIN_COPPER = 0xffd98b5f;

/** Mujin Dark exactly as the brand tokens define it. */
const MUJIN_DARK: ThemeColors = {
  canvas: "#0a0a0b",
  deep: "#050506",
  raised: "#111113",
  surface1: "#17171a",
  surface2: "#1e1e22",
  surface3: "#26262b",
  border: "#34343a",
  borderStrong: "#66666e",
  text: "#f2eee6",
  textSecondary: "#b8b3aa",
  textTertiary: "#8e8a84",
  textDisabled: "#5c5955",
  accent: "#d98b5f",
  accentHover: "#e6a27a",
  accentPressed: "#bf7246",
  onAccent: "#160d07",
  accentContainer: "#3a271d",
  onAccentContainer: "#f7e2d4",
  danger: "#f2473f",
  qualityLossless: "#d98b5f",
  qualityEnhanced: "#8fc3e0",
  qualityHigh: "#a3c98b",
  qualityLow: "#7e7a74",
  qualityClipping: "#f2473f",
};

const STATUS_SEEDS: Record<StatusFamily, Argb> = {
  enhanced: 0xff8fc3e0,
  high: 0xffa3c98b,
  brass: 0xffdaab6c,
  clipping: 0xfff2473f,
};

const QUALITY_SEEDS = {
  lossless: 0xffd98b5f,
  enhanced: 0xff8fc3e0,
  high: 0xffa3c98b,
  low: 0xff7e7a74,
  clipping: 0xfff2473f,
} as const;

const FONT_STACKS: Record<FontChoice, { display: string; body: string }> = {
  mujin: {
    display: '"Jost", "Inter", ui-sans-serif, system-ui, sans-serif',
    body: '"Inter", ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  },
  inter: {
    display: '"Inter", ui-sans-serif, system-ui, sans-serif',
    body: '"Inter", ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  },
  fraunces: {
    display: '"Fraunces", Georgia, "Times New Roman", serif',
    body: '"Inter", ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  },
  atkinson: {
    display: '"Atkinson Hyperlegible Next", "Inter", ui-sans-serif, sans-serif',
    body: '"Atkinson Hyperlegible Next", "Inter", ui-sans-serif, sans-serif',
  },
  plex: {
    display: '"IBM Plex Sans", "Inter", ui-sans-serif, sans-serif',
    body: '"IBM Plex Sans", "Inter", ui-sans-serif, sans-serif',
  },
  "roboto-flex": {
    display: '"Roboto Flex", "Inter", ui-sans-serif, sans-serif',
    body: '"Roboto Flex", "Inter", ui-sans-serif, sans-serif',
  },
};

const MONO_STACK =
  '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

const argb = (hex: string): Argb => argbFromHex(hex) ?? 0xff000000;
const hex = hexFromArgb;

/** The mode a preset shows in for this appearance and system setting. */
export function resolveMode(
  appearance: Pick<Appearance, "preset" | "color_scheme">,
  systemDark: boolean,
): ThemeMode {
  if (appearance.preset === "oled-black") return "dark";
  if (
    appearance.preset === "mujin-dark" ||
    appearance.preset === "mujin-light"
  ) {
    // The two Mujin presets are one theme in two modes: "Match system"
    // switches between them, otherwise the chosen one stays.
    if (appearance.color_scheme === "system")
      return systemDark ? "dark" : "light";
    if (appearance.color_scheme === "dark") return "dark";
    if (appearance.color_scheme === "light") return "light";
    return appearance.preset === "mujin-light" ? "light" : "dark";
  }
  if (appearance.color_scheme === "dark") return "dark";
  if (appearance.color_scheme === "light") return "light";
  return systemDark ? "dark" : "light";
}

interface NeutralTones {
  canvas: number;
  deep: number;
  raised: number;
  surface1: number;
  surface2: number;
  surface3: number;
  border: number;
  borderStrong: number;
  text: number;
  textSecondary: number;
  textTertiary: number;
  textDisabled: number;
  /** zinc 50..950 */
  ramp: number[];
}

const DARK_TONES: NeutralTones = {
  canvas: 4,
  deep: 2,
  raised: 6,
  surface1: 9,
  surface2: 12,
  surface3: 16,
  border: 23,
  borderStrong: 44,
  text: 94,
  textSecondary: 74,
  textTertiary: 60,
  textDisabled: 38,
  ramp: [94, 90, 85, 79, 74, 60, 38, 23, 16, 9, 6],
};

const LIGHT_TONES: NeutralTones = {
  canvas: 96,
  deep: 98,
  raised: 93,
  surface1: 92,
  surface2: 89,
  surface3: 86,
  border: 80,
  borderStrong: 52,
  text: 8,
  textSecondary: 30,
  textTertiary: 42,
  textDisabled: 62,
  ramp: [8, 14, 20, 26, 30, 42, 62, 80, 86, 92, 93],
};

const OLED_TONES: NeutralTones = {
  ...DARK_TONES,
  canvas: 0,
  deep: 0,
  raised: 0,
  surface1: 5,
  surface2: 8,
  surface3: 12,
  border: 18,
  borderStrong: 42,
  ramp: [94, 90, 85, 79, 74, 60, 38, 18, 12, 5, 0],
};

const HIGH_CONTRAST_DARK: NeutralTones = {
  canvas: 0,
  deep: 0,
  raised: 0,
  surface1: 5,
  surface2: 9,
  surface3: 13,
  border: 62,
  borderStrong: 88,
  text: 100,
  textSecondary: 93,
  textTertiary: 86,
  textDisabled: 64,
  ramp: [100, 97, 95, 93, 93, 86, 64, 62, 13, 5, 0],
};

const HIGH_CONTRAST_LIGHT: NeutralTones = {
  canvas: 100,
  deep: 100,
  raised: 100,
  surface1: 96,
  surface2: 93,
  surface3: 90,
  border: 38,
  borderStrong: 10,
  text: 0,
  textSecondary: 10,
  textTertiary: 20,
  textDisabled: 46,
  ramp: [0, 4, 8, 10, 10, 20, 46, 38, 90, 96, 100],
};

function neutralColors(
  neutral: TonalPalette,
  tones: NeutralTones,
  variant: TonalPalette = neutral,
): Pick<
  ThemeColors,
  | "canvas"
  | "deep"
  | "raised"
  | "surface1"
  | "surface2"
  | "surface3"
  | "border"
  | "borderStrong"
  | "text"
  | "textSecondary"
  | "textTertiary"
  | "textDisabled"
> & { ramp: Ramp } {
  return {
    canvas: hex(neutral.tone(tones.canvas)),
    deep: hex(neutral.tone(tones.deep)),
    raised: hex(neutral.tone(tones.raised)),
    surface1: hex(neutral.tone(tones.surface1)),
    surface2: hex(neutral.tone(tones.surface2)),
    surface3: hex(neutral.tone(tones.surface3)),
    border: hex(variant.tone(tones.border)),
    borderStrong: hex(variant.tone(tones.borderStrong)),
    text: hex(neutral.tone(tones.text)),
    textSecondary: hex(variant.tone(tones.textSecondary)),
    textTertiary: hex(variant.tone(tones.textTertiary)),
    textDisabled: hex(neutral.tone(tones.textDisabled)),
    ramp: tones.ramp.map((tone, index) =>
      hex((index >= 3 && index <= 7 ? variant : neutral).tone(tone)),
    ),
  };
}

/** Accent steps 50..950 around `anchor` (the step-400 colour). */
export function accentRampFor(anchor: Argb, mode: ThemeMode): Ramp {
  const hct = hctFromArgb(anchor);
  const palette = new TonalPalette(hct.hue, Math.max(hct.chroma, 8));
  const tone = hct.tone;
  const clamp = (value: number) => Math.max(3, Math.min(98, value));
  const offsets =
    mode === "dark"
      ? [null, null, null, null, 0, -9, -18, -28, -38, -46, -52]
      : [null, null, null, null, 0, 8, 16, 28, 40, 48, 52];
  const lead =
    mode === "dark"
      ? [96, 92, 84, Math.max(tone + 6, 76)]
      : [8, 14, 22, Math.min(tone - 6, 30)];
  return offsets.map((offset, index) =>
    offset === null
      ? hex(palette.tone(clamp(lead[index]!)))
      : index === 4
        ? hex(anchor)
        : hex(palette.tone(clamp(tone + offset))),
  );
}

interface Assembly {
  colors: ThemeColors;
  checks: ContrastCheck[];
}

/** Moves tones until every text and control pair passes. */
function enforceContrast(colors: ThemeColors, high: boolean): Assembly {
  const checks: ContrastCheck[] = [];
  const next = { ...colors };
  const fix = (
    key: keyof ThemeColors,
    against: (keyof ThemeColors)[],
    minimum: number,
    name: string,
  ) => {
    let value = argb(next[key]);
    let adjusted = false;
    for (const background of against) {
      const fixed = ensureContrast(value, argb(next[background]), minimum);
      if (fixed !== value) {
        value = fixed;
        adjusted = true;
      }
    }
    next[key] = hex(value);
    for (const background of against) {
      checks.push({
        name: `${name} on ${background}`,
        foreground: next[key],
        background: next[background],
        ratio:
          Math.round(contrastRatio(value, argb(next[background])) * 100) / 100,
        minimum,
        adjusted,
      });
    }
  };
  const textMinimum = high ? 15 : 7;
  const secondaryMinimum = high ? 10 : 4.5;
  const tertiaryMinimum = high ? 7 : 4.5;
  fix("text", ["canvas", "surface3"], textMinimum, "Text");
  fix(
    "textSecondary",
    ["canvas", "surface2"],
    secondaryMinimum,
    "Secondary text",
  );
  fix("textTertiary", ["canvas", "surface1"], tertiaryMinimum, "Tertiary text");
  fix("accent", ["canvas", "surface1"], high ? 7 : 4.5, "Accent");
  // Text on the accent (the Play button): pick ink or paper, then nudge.
  const accent = argb(next.accent);
  const onAccentMinimum = high ? 7 : 4.5;
  if (contrastRatio(argb(next.onAccent), accent) < onAccentMinimum) {
    next.onAccent = hex(
      ensureContrast(
        bestOn(accent, argb(next.text), argb(next.canvas)),
        accent,
        onAccentMinimum,
      ),
    );
  }
  checks.push({
    name: "Text on accent",
    foreground: next.onAccent,
    background: next.accent,
    ratio: Math.round(contrastRatio(argb(next.onAccent), accent) * 100) / 100,
    minimum: onAccentMinimum,
    adjusted: false,
  });
  fix(
    "onAccentContainer",
    ["accentContainer"],
    4.5,
    "Text on accent container",
  );
  fix("borderStrong", ["canvas"], high ? 4.5 : 3, "Strong border");
  fix("danger", ["canvas"], 4.5, "Error text");
  for (const key of [
    "qualityLossless",
    "qualityEnhanced",
    "qualityHigh",
    "qualityLow",
    "qualityClipping",
  ] as const) {
    fix(key, ["canvas"], 3, `Signal light ${key.slice(7).toLowerCase()}`);
  }
  return { colors: next, checks };
}

function qualityColors(
  mode: ThemeMode,
): Pick<
  ThemeColors,
  | "qualityLossless"
  | "qualityEnhanced"
  | "qualityHigh"
  | "qualityLow"
  | "qualityClipping"
  | "danger"
> {
  if (mode === "dark") {
    return {
      qualityLossless: MUJIN_DARK.qualityLossless,
      qualityEnhanced: MUJIN_DARK.qualityEnhanced,
      qualityHigh: MUJIN_DARK.qualityHigh,
      qualityLow: MUJIN_DARK.qualityLow,
      qualityClipping: MUJIN_DARK.qualityClipping,
      danger: MUJIN_DARK.danger,
    };
  }
  const deepen = (seed: Argb, tone: number) => {
    const hct = hctFromArgb(seed);
    return hex(new TonalPalette(hct.hue, hct.chroma).tone(tone));
  };
  return {
    qualityLossless: deepen(QUALITY_SEEDS.lossless, 48),
    qualityEnhanced: deepen(QUALITY_SEEDS.enhanced, 46),
    qualityHigh: deepen(QUALITY_SEEDS.high, 46),
    qualityLow: deepen(QUALITY_SEEDS.low, 50),
    qualityClipping: deepen(QUALITY_SEEDS.clipping, 44),
    danger: deepen(QUALITY_SEEDS.clipping, 42),
  };
}

function statusRamps(
  mode: ThemeMode,
  canvas: Argb,
): Record<StatusFamily, Ramp> {
  const ramps = {} as Record<StatusFamily, Ramp>;
  for (const family of STATUS_FAMILIES) {
    const seed = STATUS_SEEDS[family];
    const anchor = mode === "dark" ? seed : ensureContrast(seed, canvas, 4.5);
    ramps[family] = accentRampFor(anchor, mode);
  }
  return ramps;
}

function accentFamily(
  accentArgb: Argb,
  mode: ThemeMode,
): Pick<
  ThemeColors,
  | "accent"
  | "accentHover"
  | "accentPressed"
  | "accentContainer"
  | "onAccentContainer"
> {
  const hct = hctFromArgb(accentArgb);
  const palette = new TonalPalette(hct.hue, hct.chroma);
  const step = mode === "dark" ? 1 : -1;
  return {
    accent: hex(accentArgb),
    accentHover: hex(palette.tone(hct.tone + 8 * step)),
    accentPressed: hex(palette.tone(hct.tone - 8 * step)),
    accentContainer: hex(
      new TonalPalette(hct.hue, Math.min(hct.chroma, 40)).tone(
        mode === "dark" ? 24 : 90,
      ),
    ),
    onAccentContainer: hex(palette.tone(mode === "dark" ? 90 : 12)),
  };
}

/**
 * Material 3 colour roles from a source colour (scheme "tonal spot": the
 * primary keeps a calm chroma of 36, surfaces are faintly tinted).
 */
export function materialRoles(source: Argb, mode: ThemeMode) {
  const hct = hctFromArgb(source);
  const primary = new TonalPalette(hct.hue, 36);
  const secondary = new TonalPalette(hct.hue, 16);
  const tertiary = new TonalPalette((hct.hue + 60) % 360, 24);
  const neutral = new TonalPalette(hct.hue, 6);
  const neutralVariant = new TonalPalette(hct.hue, 8);
  const dark = mode === "dark";
  return {
    primary: primary.tone(dark ? 80 : 40),
    onPrimary: primary.tone(dark ? 20 : 100),
    primaryContainer: primary.tone(dark ? 30 : 90),
    onPrimaryContainer: primary.tone(dark ? 90 : 10),
    secondary: secondary.tone(dark ? 80 : 40),
    secondaryContainer: secondary.tone(dark ? 30 : 90),
    tertiary: tertiary.tone(dark ? 80 : 40),
    surface: neutral.tone(dark ? 6 : 98),
    surfaceContainerLowest: neutral.tone(dark ? 4 : 100),
    surfaceContainerLow: neutral.tone(dark ? 10 : 96),
    surfaceContainer: neutral.tone(dark ? 12 : 94),
    surfaceContainerHigh: neutral.tone(dark ? 17 : 92),
    surfaceContainerHighest: neutral.tone(dark ? 22 : 90),
    onSurface: neutral.tone(dark ? 90 : 10),
    onSurfaceVariant: neutralVariant.tone(dark ? 80 : 30),
    outline: neutralVariant.tone(dark ? 60 : 50),
    outlineVariant: neutralVariant.tone(dark ? 30 : 80),
    palettes: { primary, secondary, tertiary, neutral, neutralVariant },
  };
}

function materialColors(
  source: Argb,
  mode: ThemeMode,
): ThemeColors & { ramp: Ramp } {
  const roles = materialRoles(source, mode);
  const { neutral, neutralVariant, primary } = roles.palettes;
  const dark = mode === "dark";
  return {
    canvas: hex(roles.surface),
    deep: hex(roles.surfaceContainerLowest),
    raised: hex(roles.surfaceContainerLow),
    surface1: hex(roles.surfaceContainer),
    surface2: hex(roles.surfaceContainerHigh),
    surface3: hex(roles.surfaceContainerHighest),
    border: hex(roles.outlineVariant),
    borderStrong: hex(roles.outline),
    text: hex(roles.onSurface),
    textSecondary: hex(roles.onSurfaceVariant),
    textTertiary: hex(neutralVariant.tone(dark ? 70 : 40)),
    textDisabled: hex(neutral.tone(dark ? 38 : 62)),
    accent: hex(roles.primary),
    accentHover: hex(primary.tone(dark ? 90 : 30)),
    accentPressed: hex(primary.tone(dark ? 70 : 50)),
    onAccent: hex(roles.onPrimary),
    accentContainer: hex(roles.primaryContainer),
    onAccentContainer: hex(roles.onPrimaryContainer),
    ...qualityColors(mode),
    ramp: (dark
      ? [95, 90, 85, 80, 80, 70, 38, 30, 22, 12, 10]
      : [10, 14, 20, 26, 30, 40, 62, 80, 90, 94, 96]
    ).map((tone, index) =>
      hex((index >= 3 && index <= 7 ? neutralVariant : neutral).tone(tone)),
    ),
  };
}

/** The window material to ask the desktop shell for, if it has one. */
function nativeMaterialFor(
  appearance: Appearance,
  environment: ThemeEnvironment,
): NativeMaterial | null {
  const supported = environment.nativeMaterials ?? [];
  if (supported.includes(appearance.glass_material))
    return appearance.glass_material;
  if (supported.includes("vibrancy")) return "vibrancy";
  return null;
}

function shapeScale(appearance: Appearance): number {
  const base = appearance.corner_radius / DEFAULT_CORNER_RADIUS;
  return (
    Math.round(base * (appearance.preset === "material-you" ? 1.75 : 1) * 100) /
    100
  );
}

/** Resolves an appearance (any shape) into the full theme. */
export function resolveTheme(
  input: Appearance | unknown,
  environment: ThemeEnvironment,
): ResolvedTheme {
  const appearance = normalizeAppearance(input);
  const preset = appearance.preset;
  const mode = resolveMode(appearance, environment.systemDark);
  const high = preset === "high-contrast";
  const custom = appearance.accent ? argbFromHex(appearance.accent) : null;
  const artwork = appearance.accent_from_artwork
    ? (environment.artworkColor ?? null)
    : null;
  const system =
    preset === "material-you" && appearance.follow_system_colors
      ? (environment.systemAccent ?? null)
      : null;
  const accentSeed = system ?? artwork ?? custom ?? MUJIN_COPPER;
  const accentSource: AccentSource = system
    ? "system"
    : artwork
      ? "artwork"
      : custom
        ? "custom"
        : "brand";

  let colors: ThemeColors;
  let neutralRamp: Ramp | null = null;
  const brandNeutral = TonalPalette.fromArgb(argb(MUJIN_DARK.surface1));
  const paperNeutral = TonalPalette.fromArgb(0xfff2eee6);

  if (preset === "material-you") {
    const generated = materialColors(accentSeed, mode);
    const { ramp, ...rest } = generated;
    colors = rest;
    neutralRamp = ramp;
  } else {
    let base: ThemeColors;
    if (
      mode === "dark" &&
      (preset === "mujin-dark" ||
        preset === "mujin-light" ||
        preset === "frosted-glass")
    ) {
      base = { ...MUJIN_DARK };
    } else {
      const tones =
        preset === "oled-black"
          ? OLED_TONES
          : high
            ? mode === "dark"
              ? HIGH_CONTRAST_DARK
              : HIGH_CONTRAST_LIGHT
            : mode === "dark"
              ? DARK_TONES
              : LIGHT_TONES;
      const neutral = mode === "light" ? paperNeutral : brandNeutral;
      const generated = neutralColors(
        high ? new TonalPalette(0, 0) : neutral,
        tones,
      );
      const { ramp, ...neutrals } = generated;
      neutralRamp = ramp;
      if (preset === "oled-black") {
        // Keep the brand's ivory type on true black.
        neutrals.text = MUJIN_DARK.text;
        neutrals.textSecondary = MUJIN_DARK.textSecondary;
        neutrals.textTertiary = MUJIN_DARK.textTertiary;
        neutrals.textDisabled = MUJIN_DARK.textDisabled;
      }
      if (mode === "light" && !high) {
        neutrals.canvas = "#f2eee6";
        neutrals.text = "#0a0a0b";
      }
      base = {
        ...MUJIN_DARK,
        ...neutrals,
        ...qualityColors(mode),
      };
    }
    // The accent: the brand copper keeps its hand-made steps in dark mode.
    if (accentSource !== "brand" || mode === "light" || high) {
      const seed = ensureContrast(
        accentSeed,
        argb(base.canvas),
        high ? 7 : 4.5,
      );
      base = { ...base, ...accentFamily(seed, mode) };
    }
    colors = base;
  }

  const enforced = enforceContrast(colors, high);
  colors = enforced.colors;

  const brandAccent =
    accentSource === "brand" &&
    mode === "dark" &&
    !high &&
    preset !== "material-you";
  const accentRamp = brandAccent
    ? null
    : accentRampFor(argb(colors.accent), mode);
  const statuses =
    mode === "light" || high ? statusRamps(mode, argb(colors.canvas)) : null;
  // The literal brand neutrals already live in the stylesheet.
  if (
    mode === "dark" &&
    (preset === "mujin-dark" ||
      preset === "mujin-light" ||
      preset === "frosted-glass")
  ) {
    neutralRamp = null;
  }

  const reducedTransparency =
    appearance.transparency === "reduced" ||
    Boolean(environment.reducedTransparency);
  const glassEnabled = preset === "frosted-glass" && !reducedTransparency;
  const fontStack =
    FONT_STACKS[
      appearance.font === "mujin" && preset === "material-you"
        ? "roboto-flex"
        : appearance.font
    ];
  return {
    preset,
    label: PRESET_LABELS[preset],
    mode,
    contrast: high ? "high" : "standard",
    colors,
    neutralRamp,
    accentRamp,
    statusRamps: statuses,
    shape: {
      scale: shapeScale(appearance),
      material: preset === "material-you",
    },
    glass: {
      enabled: glassEnabled,
      native: glassEnabled ? nativeMaterialFor(appearance, environment) : null,
      panelOpacity: mode === "dark" ? 0.46 : 0.58,
      blurPx: 28,
      noise: glassEnabled,
    },
    type: {
      scale: appearance.text_scale,
      font: appearance.font,
      display: fontStack.display,
      body: fontStack.body,
      mono: MONO_STACK,
      material: preset === "material-you",
    },
    density: appearance.density,
    motion:
      appearance.motion === "reduced" || environment.reducedMotion
        ? "reduced"
        : "full",
    nowPlayingLayout: appearance.now_playing_layout,
    sidebar: appearance.sidebar,
    accentSource,
    checks: enforced.checks,
  };
}

/** The lowest ratio each check reached, for tests and the settings page. */
export function failingChecks(theme: ResolvedTheme): ContrastCheck[] {
  return theme.checks.filter((check) => check.ratio + 0.005 < check.minimum);
}
