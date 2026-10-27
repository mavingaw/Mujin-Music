import {
  type Appearance,
  getAppearance,
  normalizeAppearance,
  saveAppearance,
  appearanceConflict,
  sourceColorFromSwatch,
  type CoverSwatch,
} from "@parson/music-sdk";
import { reloadAppAsync } from "expo";
import * as SecureStore from "expo-secure-store";
import { useEffect, useRef } from "react";
import { AppState, Platform } from "react-native";

import { storedAppearance } from "@/constants/colors";
import {
  APPEARANCE_STORE_KEY,
  serializeStoredAppearance,
  type StoredAppearance,
} from "@/lib/appearance";
import { imageRequestHeaders, imageUrl } from "@/lib/runtime";
import { usePlayer } from "@/providers/player-provider";
import { useSession } from "@/providers/session-provider";

let current: StoredAppearance = storedAppearance;

export function persistAppearance(next: StoredAppearance) {
  current = next;
  const value = serializeStoredAppearance(next);
  try {
    if (Platform.OS === "web") {
      globalThis.localStorage?.setItem(APPEARANCE_STORE_KEY, value);
    } else {
      SecureStore.setItem(APPEARANCE_STORE_KEY, value);
    }
  } catch {
    // The theme still applies for this launch.
  }
}

export function currentStoredAppearance(): StoredAppearance {
  return current;
}

const same = (a: Appearance, b: Appearance) =>
  JSON.stringify(normalizeAppearance(a)) ===
  JSON.stringify(normalizeAppearance(b));

/** Restyles every screen with the stored theme (a JavaScript reload). */
export async function restyleApp() {
  try {
    await reloadAppAsync("Applying the appearance");
  } catch {
    // Older shells: the theme applies on the next launch.
  }
}

/**
 * Saves an appearance to the profile and to this phone, then restyles.
 * A save that lost a race with another device keeps that device's choices
 * and applies this one's on top.
 */
export async function applyAppearance(next: Appearance) {
  const normalized = normalizeAppearance(next);
  let revision = current.revision;
  try {
    const saved = await saveAppearance(normalized, current.revision);
    revision = saved.revision;
  } catch (error) {
    const newer = appearanceConflict(error);
    if (newer) {
      const saved = await saveAppearance(normalized, newer.revision);
      revision = saved.revision;
    }
    // Offline: keep it on this phone; the next sync uploads nothing, so
    // the profile keeps its server copy until saved again.
  }
  persistAppearance({ ...current, appearance: normalized, revision });
  await restyleApp();
}

/** The source colour of a cover, from the server's 24×24 swatch. */
export async function coverSourceColor(
  coverPath: string | null | undefined,
): Promise<number | null> {
  const uri = imageUrl(coverPath);
  if (!uri) return null;
  const swatchUri = `${uri}${uri.includes("?") ? "&" : "?"}format=swatch`;
  try {
    const response = await fetch(swatchUri, {
      headers: imageRequestHeaders(swatchUri),
    });
    if (!response.ok || response.status === 204) return null;
    return sourceColorFromSwatch((await response.json()) as CoverSwatch);
  } catch {
    return null;
  }
}

/**
 * Keeps this phone's theme in step with the profile: on sign-in and when
 * the app returns to the foreground it reads the saved appearance; a change
 * made on another device is stored and, when nothing is playing, applied
 * at once (otherwise on the next launch, so playback is never cut).
 * With colour from album art on, it remembers the playing cover's colour.
 */
export function AppearanceSync() {
  const session = useSession();
  const { current: song, isPlaying } = usePlayer();
  const playing = useRef(isPlaying);
  useEffect(() => {
    playing.current = isPlaying;
  }, [isPlaying]);
  const ready = session.phase === "ready";

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    const sync = async () => {
      try {
        const record = await getAppearance();
        if (cancelled) return;
        if (
          record.revision === current.revision &&
          same(record.appearance, current.appearance)
        ) {
          return;
        }
        if (record.revision === 0 && current.revision === 0) return;
        persistAppearance({
          ...current,
          appearance: record.appearance,
          revision: record.revision,
        });
        if (!playing.current) await restyleApp();
      } catch {
        // Offline or an older server: keep the stored theme.
      }
    };
    void sync();
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void sync();
    });
    return () => {
      cancelled = true;
      subscription.remove();
    };
  }, [ready]);

  const cover = song?.album_object?.cover_url ?? null;
  useEffect(() => {
    if (!ready || !current.appearance.accent_from_artwork || !cover) return;
    let cancelled = false;
    void coverSourceColor(cover).then((color) => {
      if (cancelled || color === null || color === current.artworkColor) return;
      persistAppearance({ ...current, artworkColor: color });
    });
    return () => {
      cancelled = true;
    };
  }, [cover, ready]);

  return null;
}
