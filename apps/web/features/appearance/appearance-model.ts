import {
  type Appearance,
  type ContrastCheck,
  type FontChoice,
  type ResolvedTheme,
  type ThemeEnvironment,
  type ThemePreset,
  resolveTheme,
  themeVariables,
} from "@parson/music-sdk";
import type { CSSProperties } from "react";

/**
 * The face each font choice is previewed in on the Font picker. Every family
 * here must have an @font-face in app/globals.css (font-faces.test.ts).
 */
export const FONT_SAMPLE_STACKS: Record<FontChoice, string> = {
  mujin: '"Jost", sans-serif',
  inter: '"Inter", sans-serif',
  fraunces: '"Fraunces", serif',
  atkinson: '"Atkinson Hyperlegible Next", sans-serif',
  plex: '"IBM Plex Sans", sans-serif',
  "roboto-flex": '"Roboto Flex", sans-serif',
};

/** Accent swatches: Mujin Copper first, then hues that hold up in every preset. */
export const ACCENT_SWATCHES: { name: string; value: string | null }[] = [
  { name: "Mujin Copper", value: null },
  { name: "Brass", value: "#c9a24a" },
  { name: "Sage", value: "#8fb07a" },
  { name: "Teal", value: "#4fa7a0" },
  { name: "Steel blue", value: "#6c9fd0" },
  { name: "Indigo", value: "#7d83e8" },
  { name: "Orchid", value: "#c27ac0" },
  { name: "Rose", value: "#e07a8a" },
  { name: "Vermilion", value: "#e2603f" },
];

export type ContrastGrade = "AAA" | "AA" | "AA large" | "Fails";

/** WCAG 2.x grade for text at this ratio. */
export function contrastGrade(ratio: number): ContrastGrade {
  if (ratio >= 7) return "AAA";
  if (ratio >= 4.5) return "AA";
  if (ratio >= 3) return "AA large";
  return "Fails";
}

/**
 * The few checks worth showing a person, in reading order: body text,
 * quiet text, the accent as text, and text on the Play button.
 */
export function headlineChecks(theme: ResolvedTheme): ContrastCheck[] {
  const wanted = [
    "Text on canvas",
    "Secondary text on canvas",
    "Tertiary text on canvas",
    "Accent on canvas",
    "Text on accent",
  ];
  return wanted
    .map((name) => theme.checks.find((check) => check.name === name))
    .filter((check): check is ContrastCheck => Boolean(check));
}

/** Inline CSS variables that draw an element in another preset's colours. */
export function previewStyle(
  preset: ThemePreset,
  appearance: Appearance,
  environment: ThemeEnvironment,
): { theme: ResolvedTheme; style: CSSProperties } {
  const theme = resolveTheme({ ...appearance, preset }, environment);
  return {
    theme,
    style: themeVariables(theme) as unknown as CSSProperties,
  };
}

/** "Saved to your profile" style status text. */
export function syncLabel(
  status: "local" | "saving" | "saved" | "error",
  signedIn: boolean,
): string {
  if (!signedIn) return "Applies on this device";
  switch (status) {
    case "saving":
      return "Saving to your profile…";
    case "saved":
      return "Saved to your profile · follows you to Mujin Music on the web, desktop and phone";
    case "error":
      return "Not saved yet · Mujin Music will retry when the server answers";
    default:
      return "Applies on this device";
  }
}
