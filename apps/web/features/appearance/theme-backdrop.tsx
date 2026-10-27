"use client";

import { usePlayer } from "@/features/player/player-api";
import { useAppearance } from "./theme-provider";

/**
 * Frosted Glass: what the translucent panels blur. In a browser it is the
 * playing album's cover, heavily blurred, over the accent glow; in the
 * desktop app the native Mica, Acrylic or vibrancy shows through instead
 * and only a faint wash of the cover remains. A fine grain sits on top so
 * the glass reads as a material, not a flat tint.
 */
export function ThemeBackdrop() {
  const { theme } = useAppearance();
  const { imageSrc } = usePlayer();
  if (!theme.glass.enabled) return null;
  return (
    <>
      <div aria-hidden="true" className="mj-wallpaper">
        {imageSrc ? (
          <div
            className="mj-wallpaper-art"
            style={{ backgroundImage: `url("${imageSrc}")` }}
          />
        ) : null}
      </div>
      {theme.glass.noise ? (
        <div aria-hidden="true" className="mj-noise" />
      ) : null}
    </>
  );
}
