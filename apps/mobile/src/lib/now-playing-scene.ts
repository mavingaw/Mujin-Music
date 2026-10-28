import type { ResolvedTheme } from "@parson/music-sdk";
import { type MobilePalette, tintOf, withAlpha } from "./appearance";

/**
 * How the phone's Now Playing screen is painted for the theme's backdrop
 * (the same rule as the web app's `nowPlayingScene`):
 * - artwork: the blurred cover over the theme's surfaces;
 * - glass: a brighter blurred cover under translucent panels;
 * - black: true #000 and no colour from the cover (OLED Black);
 * - solid: the plain canvas with opaque, bordered panels (High Contrast).
 */
export interface PhoneNowPlayingScene {
  /** The page gradient, top to bottom. */
  page: [string, string, string];
  /** Draw the cover behind everything (blurred, or sharp when immersive). */
  showArt: boolean;
  artOpacity: number;
  /** Scrim stops over the cover. */
  scrims: [number, number, number];
  /** The cover fills the screen (immersive layout). */
  immersive: boolean;
  /** The Up next card and the Queue / Lyrics / Sound buttons. */
  card: {
    backgroundColor: string;
    borderColor: string;
    borderWidth: number;
  };
  button: {
    backgroundColor: string;
    borderColor: string;
    borderWidth: number;
  };
}

export function phoneNowPlayingScene(
  theme: Pick<ResolvedTheme, "backdrop" | "nowPlayingLayout">,
  palette: MobilePalette,
  hairline: number,
): PhoneNowPlayingScene {
  const wantsImmersive = theme.nowPlayingLayout === "immersive";
  const plainCard = {
    backgroundColor:
      palette.mode === "light"
        ? withAlpha(palette.elevated, 0.6)
        : "rgba(0,0,0,0.25)",
    borderColor: palette.border,
    borderWidth: hairline,
  };
  const plainButton = {
    backgroundColor: tintOf(palette, 0.1),
    borderColor: "transparent",
    borderWidth: 0,
  };
  switch (theme.backdrop) {
    case "solid": {
      const solid = {
        backgroundColor: palette.surface,
        borderColor: palette.borderStrong,
        borderWidth: 2,
      };
      return {
        page: [palette.background, palette.background, palette.background],
        showArt: false,
        artOpacity: 0,
        scrims: [0, 0, 0],
        // Text never sits on the cover in High Contrast.
        immersive: false,
        card: solid,
        button: solid,
      };
    }
    case "black":
      return {
        page: ["#000000", "#000000", "#000000"],
        // Only the immersive layout shows the cover, fading to black.
        showArt: wantsImmersive,
        artOpacity: 0.6,
        scrims: [0.2, 0.45, 1],
        immersive: wantsImmersive,
        card: { ...plainCard, backgroundColor: "#000000" },
        button: plainButton,
      };
    case "glass": {
      const glass = {
        backgroundColor: withAlpha(
          palette.elevated,
          palette.mode === "dark" ? 0.46 : 0.58,
        ),
        borderColor: tintOf(palette, 0.16),
        borderWidth: hairline,
      };
      return {
        page: [palette.surfaceStrong, palette.background, palette.deep],
        showArt: true,
        artOpacity: wantsImmersive ? 0.6 : 0.8,
        scrims: wantsImmersive ? [0.15, 0.35, 0.9] : [0.1, 0.35, 0.7],
        immersive: wantsImmersive,
        card: glass,
        button: glass,
      };
    }
    default:
      return {
        page: [palette.surfaceStrong, palette.background, palette.deep],
        showArt: true,
        artOpacity: 0.5,
        scrims: wantsImmersive ? [0.15, 0.35, 0.97] : [0.25, 0.7, 0.96],
        immersive: wantsImmersive,
        card: plainCard,
        button: plainButton,
      };
  }
}
