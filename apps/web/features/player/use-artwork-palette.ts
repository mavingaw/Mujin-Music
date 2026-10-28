"use client";

import { useEffect, useState } from "react";
import {
  fallbackPalette,
  paletteFromPixels,
  type ArtworkPalette,
} from "./artwork-palette";

const cache = new Map<string, ArtworkPalette>();

/**
 * Samples the artwork at 32×32 in a canvas and picks its colour. Artwork
 * the browser won't let us read (another origin without CORS) keeps the
 * copper fallback.
 */
export function useArtworkPalette(src: string | null | undefined) {
  const [palette, setPalette] = useState<ArtworkPalette>(
    () => (src && cache.get(src)) || fallbackPalette,
  );
  useEffect(() => {
    if (!src) {
      setPalette(fallbackPalette);
      return;
    }
    const known = cache.get(src);
    if (known) {
      setPalette(known);
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
        canvas.width = 32;
        canvas.height = 32;
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (!context) return;
        context.drawImage(image, 0, 0, 32, 32);
        const result = paletteFromPixels(
          context.getImageData(0, 0, 32, 32).data,
        );
        cache.set(src, result);
        setPalette(result);
      } catch {
        setPalette(fallbackPalette);
      }
    };
    image.onerror = () => {
      if (!cancelled) setPalette(fallbackPalette);
    };
    image.src = src;
    return () => {
      cancelled = true;
    };
  }, [src]);
  return palette;
}
