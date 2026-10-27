import { describe, expect, test } from "bun:test";
import {
  contrastRatio,
  argbFromHex,
  DEFAULT_APPEARANCE,
  THEME_PRESETS,
} from "@parson/music-sdk";
import {
  argbFromAndroidColor,
  mobilePalette,
  mobileRadius,
  parseStoredAppearance,
  resolveMobileTheme,
  serializeStoredAppearance,
  tintOf,
  withAlpha,
} from "./appearance";

const ratio = (a: string, b: string) =>
  contrastRatio(argbFromHex(a)!, argbFromHex(b)!);

describe("phone appearance", () => {
  test("the stored appearance survives a round trip and bad data", () => {
    const stored = {
      appearance: { ...DEFAULT_APPEARANCE, preset: "oled-black" as const },
      revision: 6,
      artworkColor: 0xff336699,
    };
    expect(parseStoredAppearance(serializeStoredAppearance(stored))).toEqual(
      stored,
    );
    expect(parseStoredAppearance("{broken").appearance).toEqual(
      DEFAULT_APPEARANCE,
    );
    expect(parseStoredAppearance(null).revision).toBe(0);
  });

  test("Mujin Dark keeps the brand palette on the phone", () => {
    const theme = resolveMobileTheme(parseStoredAppearance(null), {
      systemDark: true,
    });
    const palette = mobilePalette(theme);
    expect(palette.background).toBe("#0a0a0b");
    expect(palette.accent).toBe("#d98b5f");
    expect(palette.mode).toBe("dark");
    expect(mobileRadius(theme)).toBe(16);
  });

  test("every preset is readable on the phone in light and dark", () => {
    for (const preset of THEME_PRESETS) {
      for (const color_scheme of ["dark", "light"] as const) {
        const palette = mobilePalette(
          resolveMobileTheme(
            {
              appearance: { ...DEFAULT_APPEARANCE, preset, color_scheme },
              revision: 0,
              artworkColor: null,
            },
            { systemDark: true },
          ),
        );
        expect(ratio(palette.text, palette.background)).toBeGreaterThan(7);
        expect(ratio(palette.secondary, palette.background)).toBeGreaterThan(
          4.5,
        );
        expect(ratio(palette.accentText, palette.accent)).toBeGreaterThan(4.5);
      }
    }
  });

  test("Android 12's wallpaper colour seeds Material You", () => {
    // Android hands colours over as signed 32-bit integers.
    expect(argbFromAndroidColor(-10011977)).toBe(0xff673ab7);
    expect(argbFromAndroidColor(undefined)).toBeNull();
    const theme = resolveMobileTheme(
      {
        appearance: { ...DEFAULT_APPEARANCE, preset: "material-you" },
        revision: 0,
        artworkColor: null,
      },
      { systemDark: true, systemAccent: argbFromAndroidColor(-10011977) },
    );
    expect(theme.accentSource).toBe("system");
  });

  test("washes follow the theme's text colour", () => {
    expect(tintOf({ text: "#f2eee6" }, 0.08)).toBe("rgba(242,238,230,0.08)");
    expect(withAlpha("#0a0a0b", 0.5)).toBe("rgba(10,10,11,0.5)");
  });
});
