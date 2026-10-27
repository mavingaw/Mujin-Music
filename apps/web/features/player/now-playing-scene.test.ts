import { describe, expect, test } from "bun:test";
import { fallbackPalette } from "./artwork-palette";
import { nowPlayingScene } from "./now-playing-scene";

const brown = fallbackPalette;
const scene = (
  backdrop: Parameters<typeof nowPlayingScene>[0]["backdrop"],
  options: Partial<Parameters<typeof nowPlayingScene>[0]> = {},
) =>
  nowPlayingScene({
    backdrop,
    dim: false,
    layout: "art-left",
    light: false,
    palette: brown,
    ...options,
  });

describe("Now Playing scene", () => {
  test("OLED Black is true black with no colour from the cover", () => {
    const black = scene("black");
    expect(black.base).toBe("#000000");
    expect(black.showArt).toBeFalse();
    expect(black.wash).toBeNull();
    expect(black.vignette).toBeNull();
    // No brown glow under the cover either.
    expect(black.coverShadow).not.toContain("217");
    expect(JSON.stringify(black)).not.toContain("22 14 10");
    // Immersive keeps the cover but fades it to pure black.
    const immersive = scene("black", { layout: "immersive" });
    expect(immersive.showArt).toBeTrue();
    expect(immersive.wash).toContain("rgb(0 0 0 / 1)");
    expect(immersive.wash).not.toContain("22 14 10");
  });

  test("the artwork backdrop is warmed by the cover", () => {
    const art = scene("artwork");
    expect(art.showArt).toBeTrue();
    expect(art.base).toContain("22 14 10");
    expect(art.panels).toBe("plain");
  });

  test("Frosted Glass shows more of the cover under glass panels", () => {
    const glass = scene("glass");
    expect(glass.panels).toBe("glass");
    expect(glass.showArt).toBeTrue();
    expect(glass.dim).toBe("bg-black/10");
    expect(scene("artwork").dim).toBe("bg-black/35");
  });

  test("High Contrast is solid and never puts text over the art", () => {
    const solid = scene("solid", { layout: "immersive" });
    expect(solid.showArt).toBeFalse();
    expect(solid.panels).toBe("solid");
    expect(solid.layout).toBe("art-centred");
    expect(solid.base).toBe("var(--color-mujin-base)");
    expect(scene("solid").layout).toBe("art-left");
  });
});
