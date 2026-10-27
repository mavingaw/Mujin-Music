/**
 * A resolved theme as CSS custom properties and `<html>` data attributes.
 * The web app's stylesheet reads every colour, radius, font and spacing
 * from these variables (Tailwind's `zinc`, `white`, `black` and accent
 * families included), so one `:root { ... }` block re-themes every
 * surface without touching a component.
 */

import { argbFromHex, rgbTriplet } from "./color";
import { hctFromArgb } from "./hct";
import type { ResolvedTheme, Ramp, StatusFamily } from "./engine";

const STEPS = [
  "50",
  "100",
  "200",
  "300",
  "400",
  "500",
  "600",
  "700",
  "800",
  "900",
  "950",
];

const ACCENT_FAMILIES = ["indigo", "violet", "purple", "fuchsia", "pink"];
const STATUS_TAILWIND: Record<StatusFamily, string[]> = {
  enhanced: ["sky", "blue", "cyan"],
  high: ["teal", "emerald", "green", "lime"],
  brass: ["amber", "yellow", "orange"],
  clipping: ["red", "rose"],
};

function triplet(hex: string): string {
  return rgbTriplet(argbFromHex(hex) ?? 0xff000000);
}

/** `H S% L%` for the shadcn-style tokens. */
export function hslTriplet(hex: string): string {
  const argb = argbFromHex(hex) ?? 0xff000000;
  const r = ((argb >> 16) & 255) / 255;
  const g = ((argb >> 8) & 255) / 255;
  const b = (argb & 255) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const light = (max + min) / 2;
  let hue = 0;
  let saturation = 0;
  if (max !== min) {
    const delta = max - min;
    saturation = light > 0.5 ? delta / (2 - max - min) : delta / (max + min);
    hue =
      max === r
        ? (g - b) / delta + (g < b ? 6 : 0)
        : max === g
          ? (b - r) / delta + 2
          : (r - g) / delta + 4;
    hue *= 60;
  }
  return `${Math.round(hue)} ${Math.round(saturation * 100)}% ${Math.round(light * 100)}%`;
}

function rampVariables(
  families: string[],
  ramp: Ramp,
  into: Record<string, string>,
) {
  for (const family of families) {
    ramp.forEach((value, index) => {
      into[`--color-${family}-${STEPS[index]}`] = value;
    });
  }
}

/** Every custom property the theme sets, name to value. */
export function themeVariables(theme: ResolvedTheme): Record<string, string> {
  const c = theme.colors;
  const vars: Record<string, string> = {
    "--color-mujin-base": c.canvas,
    "--color-mujin-deep": c.deep,
    "--color-mujin-raised": c.raised,
    "--color-mujin-surface-1": c.surface1,
    "--color-mujin-surface-2": c.surface2,
    "--color-mujin-surface-3": c.surface3,
    "--color-mujin-border": c.border,
    "--color-mujin-border-strong": c.borderStrong,
    "--color-mujin-copper": c.accent,
    "--color-mujin-copper-hover": c.accentHover,
    "--color-mujin-copper-pressed": c.accentPressed,
    "--color-mujin-on-copper": c.onAccent,
    "--color-mujin-text": c.text,
    "--color-mujin-text-secondary": c.textSecondary,
    "--color-mujin-text-tertiary": c.textTertiary,
    "--color-mujin-text-disabled": c.textDisabled,
    "--color-mujin-accent-container": c.accentContainer,
    "--color-mujin-on-accent-container": c.onAccentContainer,
    "--color-quality-lossless": c.qualityLossless,
    "--color-quality-enhanced": c.qualityEnhanced,
    "--color-quality-high": c.qualityHigh,
    "--color-quality-low": c.qualityLow,
    "--color-quality-clipping": c.qualityClipping,
    "--mujin-quality": c.qualityLossless,
    "--color-black": c.canvas,
    "--color-white": c.text,
    // Triplets for rgb(var(--x) / alpha) in hand-written CSS.
    "--mj-ink": triplet(c.text),
    "--mj-canvas-rgb": triplet(c.canvas),
    "--mj-deep-rgb": triplet(c.deep),
    "--mj-raised-rgb": triplet(c.raised),
    "--mj-surface-rgb": triplet(c.surface1),
    "--mj-surface-2-rgb": triplet(c.surface2),
    "--mj-accent-rgb": triplet(c.accent),
    // shadcn-style tokens used by the dialog, menu and form primitives.
    "--background": hslTriplet(c.canvas),
    "--foreground": hslTriplet(c.text),
    "--card": hslTriplet(c.surface1),
    "--card-foreground": hslTriplet(c.text),
    "--popover": hslTriplet(c.surface2),
    "--popover-foreground": hslTriplet(c.text),
    "--primary": hslTriplet(c.accent),
    "--primary-foreground": hslTriplet(c.onAccent),
    "--secondary": hslTriplet(c.surface3),
    "--secondary-foreground": hslTriplet(c.text),
    "--muted": hslTriplet(c.surface3),
    "--muted-foreground": hslTriplet(c.textSecondary),
    "--accent": hslTriplet(c.surface3),
    "--accent-foreground": hslTriplet(c.text),
    "--destructive": hslTriplet(c.danger),
    "--destructive-foreground": hslTriplet(
      theme.mode === "dark" ? c.text : c.canvas,
    ),
    "--border": hslTriplet(c.border),
    "--input": hslTriplet(c.border),
    "--ring": hslTriplet(c.accent),
    "--mj-radius-scale": String(theme.shape.scale),
    "--mj-text-scale": String(theme.type.scale),
    "--font-sans": theme.type.body,
    "--font-display": theme.type.display,
    "--font-mono": theme.type.mono,
    "--spacing": theme.density === "compact" ? "0.225rem" : "0.25rem",
    "--mj-glass-opacity": String(theme.glass.panelOpacity),
    "--mj-glass-blur": `${theme.glass.blurPx}px`,
    "--mj-accent-hue": String(
      Math.round(hctFromArgb(argbFromHex(c.accent) ?? 0).hue),
    ),
  };
  if (theme.neutralRamp) {
    rampVariables(
      ["zinc", "neutral", "stone", "gray", "slate"],
      theme.neutralRamp,
      vars,
    );
  }
  if (theme.accentRamp) rampVariables(ACCENT_FAMILIES, theme.accentRamp, vars);
  if (theme.statusRamps) {
    for (const [family, names] of Object.entries(STATUS_TAILWIND)) {
      rampVariables(names, theme.statusRamps[family as StatusFamily], vars);
    }
  }
  return vars;
}

/** The theme as one `:root { ... }` rule. */
export function themeCss(theme: ResolvedTheme, selector = ":root"): string {
  const body = Object.entries(themeVariables(theme))
    .map(([name, value]) => `${name}:${value};`)
    .join("");
  return `${selector}{${body}color-scheme:${theme.mode};}`;
}

/** Attributes for `<html>`: CSS hooks for shape, glass, density and layout. */
export function themeAttributes(
  theme: ResolvedTheme,
): Record<string, string | null> {
  return {
    "data-theme": theme.preset,
    "data-mode": theme.mode,
    "data-contrast": theme.contrast === "high" ? "high" : null,
    "data-glass": theme.glass.enabled ? "on" : null,
    "data-native-material": theme.glass.native,
    "data-shape": theme.shape.material ? "material" : null,
    "data-density": theme.density,
    "data-sidebar": theme.sidebar,
    "data-np-layout": theme.nowPlayingLayout,
    "data-motion": theme.motion === "reduced" ? "reduced" : null,
    "data-font": theme.type.font,
  };
}
