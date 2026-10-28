import { describe, expect, test } from "bun:test";
import {
  DEFAULT_APPEARANCE,
  type Appearance,
  type ThemePreset,
} from "@parson/music-sdk";
import { mobilePalette, resolveMobileTheme } from "./appearance";
import { phoneNowPlayingScene } from "./now-playing-scene";

function sceneFor(preset: ThemePreset, extra: Partial<Appearance> = {}) {
  const theme = resolveMobileTheme(
    {
      appearance: { ...DEFAULT_APPEARANCE, preset, ...extra },
      revision: 0,
      artworkColor: null,
    },
    { systemDark: true },
  );
  const palette = mobilePalette(theme);
  return { scene: phoneNowPlayingScene(theme, palette, 0.5), palette };
}

describe("phone Now Playing follows the preset", () => {
  test("OLED Black is true black with no cover wash", () => {
    const { scene } = sceneFor("oled-black");
    expect(scene.page).toEqual(["#000000", "#000000", "#000000"]);
    expect(scene.showArt).toBeFalse();
    expect(scene.card.backgroundColor).toBe("#000000");
    // Colour from artwork brings the cover back.
    expect(
      sceneFor("oled-black", { accent_from_artwork: true }).scene.showArt,
    ).toBeTrue();
    // Immersive still shows the cover, fading to black.
    const immersive = sceneFor("oled-black", {
      now_playing_layout: "immersive",
    }).scene;
    expect(immersive.showArt).toBeTrue();
    expect(immersive.scrims[2]).toBe(1);
  });

  test("Frosted Glass puts translucent panels over a brighter blur", () => {
    const glass = sceneFor("frosted-glass").scene;
    const dark = sceneFor("mujin-dark").scene;
    expect(glass.artOpacity).toBeGreaterThan(dark.artOpacity);
    expect(glass.card.backgroundColor).toMatch(/^rgba\(.+,0\.46\)$/);
    expect(glass.button.backgroundColor).toBe(glass.card.backgroundColor);
    expect(glass.card.borderWidth).toBeGreaterThan(0);
  });

  test("High Contrast uses the plain canvas and opaque, bordered panels", () => {
    const { scene, palette } = sceneFor("high-contrast", {
      now_playing_layout: "immersive",
    });
    expect(scene.showArt).toBeFalse();
    expect(scene.immersive).toBeFalse();
    expect(scene.page).toEqual([
      palette.background,
      palette.background,
      palette.background,
    ]);
    expect(scene.card).toEqual({
      backgroundColor: palette.surface,
      borderColor: palette.borderStrong,
      borderWidth: 2,
    });
  });
});
