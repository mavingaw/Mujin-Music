import { Appearance as SystemAppearance, Platform } from "react-native";
import * as SecureStore from "expo-secure-store";
import { getSystemColors } from "../../modules/parson-system-colors";
import {
  APPEARANCE_STORE_KEY,
  argbFromAndroidColor,
  mobilePalette,
  mobileRadius,
  parseStoredAppearance,
  resolveMobileTheme,
  tintOf,
  withAlpha,
} from "@/lib/appearance";

/**
 * Mujin Music's colours on the phone, from the profile's theme (Settings →
 * Appearance): Mujin Dark by default, or Mujin Light, Material You (from the
 * phone's wallpaper on Android 12+, the playing cover or a chosen colour),
 * Frosted Glass, OLED Black or High Contrast.
 *
 * The theme is read synchronously before any screen builds its styles, so
 * every StyleSheet in the app is drawn in it; applying a new theme in
 * Settings reloads the app's JavaScript to restyle every screen at once.
 */
function readStored() {
  try {
    if (Platform.OS === "web") {
      return parseStoredAppearance(
        globalThis.localStorage?.getItem(APPEARANCE_STORE_KEY) ?? null,
      );
    }
    return parseStoredAppearance(SecureStore.getItem(APPEARANCE_STORE_KEY));
  } catch {
    return parseStoredAppearance(null);
  }
}

export const storedAppearance = readStored();

/** The wallpaper seed colour on Android 12+ (Material You). */
export const systemAccent =
  Platform.OS === "android"
    ? argbFromAndroidColor(getSystemColors()?.accent1)
    : null;

export const activeTheme = resolveMobileTheme(storedAppearance, {
  systemDark: SystemAppearance.getColorScheme() !== "light",
  systemAccent,
  reducedTransparency: false,
});

/** Mujin Music palette (brand tokens, or the profile's chosen theme). */
export const palette = mobilePalette(activeTheme);

/** Text colour at `alpha`, for washes and hairlines in the active theme. */
export const tint = (alpha: number) => tintOf(palette, alpha);

/** A palette colour at `opacity`. */
export const alpha = (color: string, opacity: number) =>
  withAlpha(color, opacity);

export const layout = {
  horizontal: activeTheme.density === "compact" ? 16 : 20,
  tabBar: 76,
  miniPlayer: 68,
  radius: mobileRadius(activeTheme),
} as const;

/**
 * Brand type. Jost (SIL OFL 1.1, bundled under assets/fonts/jost) for
 * display and section titles; body text stays on the platform face.
 */
export const fonts = {
  display: "Jost-Light",
  title: "Jost-Regular",
  titleMedium: "Jost-Medium",
} as const;

/** Small caps label above a title ("DAILY PICKS"). */
export const eyebrow = {
  color: palette.tertiary,
  fontSize: Math.round(11 * activeTheme.type.scale),
  fontWeight: "600",
  letterSpacing: 1.6,
  textTransform: "uppercase",
} as const;
