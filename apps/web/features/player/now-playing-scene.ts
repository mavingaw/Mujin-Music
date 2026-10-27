import type { ArtBackdrop, NowPlayingLayout } from "@parson/music-sdk";
import { rgbString, type ArtworkPalette } from "./artwork-palette";

/**
 * How Now Playing is painted for the active theme's backdrop:
 * - artwork: the blurred cover, warmed with its own colour (Mujin Dark/Light,
 *   Material You, and OLED Black with colour from artwork on);
 * - glass: the blurred cover under translucent, blurred panels;
 * - black: true #000 with no colour from the cover (OLED Black);
 * - solid: the plain canvas with opaque, bordered panels (High Contrast).
 */
export interface NowPlayingScene {
  backdrop: ArtBackdrop;
  /** The layout actually drawn (High Contrast never puts text on the art). */
  layout: NowPlayingLayout;
  /** The base colour under everything. */
  base: string | undefined;
  /** Draw the cover (blurred, or sharp for immersive). */
  showArt: boolean;
  /** Overlay gradients over the cover, or null for none. */
  wash: string | null;
  /** Darkening layer over the art (Tailwind classes), or null. */
  dim: string | null;
  /** Vignette at the edges, or null. */
  vignette: string | null;
  /** How the info column, Up next card and transport are carried. */
  panels: "plain" | "glass" | "solid";
  /** The shadow under the cover. */
  coverShadow: string;
}

const NEUTRAL_SHADOW =
  "0 0 0 1px rgb(var(--mj-ink) / 0.08), 0 30px 60px -30px rgb(0 0 0 / 0.9)";

export function nowPlayingScene({
  backdrop,
  dim,
  layout,
  light,
  palette,
}: {
  backdrop: ArtBackdrop;
  /** The queue is open over the art. */
  dim: boolean;
  layout: NowPlayingLayout;
  light: boolean;
  palette: ArtworkPalette;
}): NowPlayingScene {
  const drawnLayout =
    backdrop === "solid" && layout === "immersive" ? "art-centred" : layout;
  const immersive = drawnLayout === "immersive" && !dim;

  if (backdrop === "solid") {
    return {
      backdrop,
      layout: drawnLayout,
      base: "var(--color-mujin-base)",
      showArt: false,
      wash: null,
      dim: null,
      vignette: null,
      panels: "solid",
      coverShadow: "0 0 0 2px var(--color-mujin-border-strong)",
    };
  }

  if (backdrop === "black") {
    const black = (alpha: number) => `rgb(0 0 0 / ${alpha})`;
    return {
      backdrop,
      layout: drawnLayout,
      base: "#000000",
      // Only the immersive layout shows the cover itself; it fades to black.
      showArt: immersive,
      wash: immersive
        ? `linear-gradient(to top, ${black(1)} 8%, ${black(0.75)} 38%, ${black(0.15)} 70%, ${black(0.4)} 100%), linear-gradient(to right, ${black(0.8)} 0%, transparent 55%)`
        : null,
      dim: null,
      vignette: null,
      panels: "plain",
      coverShadow: NEUTRAL_SHADOW,
    };
  }

  const shadeAt = (alpha: number) =>
    light
      ? `rgb(var(--mj-canvas-rgb) / ${alpha})`
      : rgbString(palette.deep, alpha);
  const glass = backdrop === "glass";
  return {
    backdrop,
    layout: drawnLayout,
    base: light ? undefined : rgbString(palette.deep),
    showArt: true,
    wash: immersive
      ? `linear-gradient(to top, ${shadeAt(0.96)} 8%, ${shadeAt(0.7)} 38%, ${shadeAt(0.15)} 70%, ${shadeAt(0.35)} 100%), linear-gradient(to right, ${shadeAt(0.75)} 0%, transparent 55%)`
      : `radial-gradient(70% 60% at 28% 45%, ${rgbString(palette.glow, light ? 0.18 : 0.28)}, transparent 70%), radial-gradient(120% 100% at 50% 110%, ${shadeAt(glass ? 0.6 : 0.95)} 20%, transparent 70%)`,
    // Glass lets more of the cover through: the panels carry the contrast.
    dim: dim
      ? "bg-black/60"
      : immersive
        ? "bg-black/10"
        : glass
          ? "bg-black/10"
          : "bg-black/35",
    vignette: immersive
      ? null
      : `radial-gradient(140% 100% at 50% 40%, transparent 45%, ${light ? "rgb(var(--mj-canvas-rgb) / 0.85)" : `rgba(5,5,6,${glass ? 0.55 : 0.85})`} 100%)`,
    panels: glass ? "glass" : "plain",
    coverShadow: `0 0 0 1px rgb(var(--mj-ink) / 0.08), 0 50px 120px -40px ${rgbString(palette.glow, 0.55)}, 0 30px 60px -30px rgb(0 0 0 / 0.9)`,
  };
}
