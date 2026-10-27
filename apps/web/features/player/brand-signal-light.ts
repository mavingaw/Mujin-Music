"use client";

import { type SignalPath } from "@parson/music-sdk";
import { signalLightColor } from "./themed-signal";
import { useEffect } from "react";

/**
 * The colour of the logo's signal dot for the playing track: the same light
 * as the signal path (copper when lossless), or null when nothing is known
 * and the dot keeps its default copper.
 */
export function brandSignalColor(
  path: Pick<SignalPath, "quality" | "clipping"> | null | undefined,
) {
  return path ? signalLightColor(path) : null;
}

/** Lights every Mujin Music logo on screen with the signal-path colour. */
export function useBrandSignalLight(
  path: Pick<SignalPath, "quality" | "clipping"> | null | undefined,
) {
  const color = brandSignalColor(path);
  useEffect(() => {
    const root = document.documentElement;
    if (color) root.style.setProperty("--mujin-quality", color);
    else root.style.removeProperty("--mujin-quality");
    return () => {
      root.style.removeProperty("--mujin-quality");
    };
  }, [color]);
}
