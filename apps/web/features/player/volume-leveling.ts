"use client";

import {
  decibelsToGain,
  gainToDecibels,
  volumeLevelingGain,
  type Loudness,
  type VolumeLevelingMode,
  type VolumeLevelingResult,
} from "@parson/music-sdk";
import { useCallback, useSyncExternalStore } from "react";

const STORAGE_KEY = "parson.volume-leveling";
const MODES: readonly VolumeLevelingMode[] = ["off", "auto", "track", "album"];
const listeners = new Set<() => void>();

export const volumeLevelingModeLabels: Record<VolumeLevelingMode, string> = {
  off: "Off",
  auto: "Auto",
  track: "Track",
  album: "Album",
};

export const volumeLevelingModes = MODES;

export function parseVolumeLevelingMode(
  value: string | null | undefined,
): VolumeLevelingMode {
  return MODES.includes(value as VolumeLevelingMode)
    ? (value as VolumeLevelingMode)
    : "off";
}

function readMode(): VolumeLevelingMode {
  try {
    return parseVolumeLevelingMode(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return "off";
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  const onStorage = (event: StorageEvent) => {
    if (event.key === STORAGE_KEY) listener();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", onStorage);
  };
}

/** Volume leveling mode, remembered per browser. */
export function useVolumeLevelingMode() {
  const mode = useSyncExternalStore(subscribe, readMode, () => "off" as const);
  const setMode = useCallback((next: VolumeLevelingMode) => {
    try {
      window.localStorage.setItem(STORAGE_KEY, next);
    } catch {}
    for (const listener of listeners) listener();
  }, []);
  return [mode, setMode] as const;
}

/**
 * Whether the current track plays inside an album run, so `auto` leveling
 * keeps the album's own dynamics: a neighbour in the queue comes from the
 * same album.
 */
export function playsInAlbumContext(
  albumIds: readonly string[],
  index: number,
): boolean {
  const current = albumIds[index];
  if (!current) return false;
  return albumIds[index - 1] === current || albumIds[index + 1] === current;
}

export type ElementLeveling = {
  result: VolumeLevelingResult | null;
  /** Linear gain multiplied into the element volume. */
  gain: number;
  /** Leveling is on but the track has no tags. */
  unavailable: boolean;
};

/**
 * Leveling for an `<audio>` element, whose volume cannot exceed 1: positive
 * gains are limited to the headroom the volume slider leaves.
 */
export function elementLeveling(
  loudness: Loudness | null | undefined,
  mode: VolumeLevelingMode,
  volume: number,
  albumContext: boolean,
): ElementLeveling {
  if (mode === "off") return { result: null, gain: 1, unavailable: false };
  const headroom = volume > 0 ? -gainToDecibels(Math.min(1, volume)) : 0;
  const result = volumeLevelingGain(loudness, mode, {
    albumContext,
    maxGainDb: headroom,
  });
  if (!result) return { result: null, gain: 1, unavailable: true };
  return { result, gain: decibelsToGain(result.appliedDb), unavailable: false };
}
