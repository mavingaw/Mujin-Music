"use client";

import { sourceColorFromPixels } from "@parson/music-sdk";
import { useEffect, useState } from "react";

const cache = new Map<string, number>();

/**
 * The playing album's source colour for Material You and "colour from
 * artwork": the cover is sampled at 64×64 in a canvas and scored the way
 * Material's colour utilities score a wallpaper. Null while there is no
 * cover or it cannot be read (another origin without CORS).
 */
export function useArtworkSourceColor(
  src: string | null | undefined,
  enabled: boolean,
): number | null {
  const [color, setColor] = useState<number | null>(() =>
    src ? (cache.get(src) ?? null) : null,
  );
  useEffect(() => {
    if (!enabled || !src || src.startsWith("data:image/svg")) {
      setColor(null);
      return;
    }
    const known = cache.get(src);
    if (known !== undefined) {
      setColor(known);
      return;
    }
    let cancelled = false;
    const image = new window.Image();
    image.crossOrigin = "anonymous";
    image.decoding = "async";
    image.onload = () => {
      if (cancelled) return;
      try {
        const canvas = document.createElement("canvas");
        canvas.width = 64;
        canvas.height = 64;
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (!context) return;
        context.drawImage(image, 0, 0, 64, 64);
        const source = sourceColorFromPixels(
          context.getImageData(0, 0, 64, 64).data,
        );
        cache.set(src, source);
        setColor(source);
      } catch {
        setColor(null);
      }
    };
    image.onerror = () => {
      if (!cancelled) setColor(null);
    };
    image.src = src;
    return () => {
      cancelled = true;
    };
  }, [enabled, src]);
  return enabled ? color : null;
}
