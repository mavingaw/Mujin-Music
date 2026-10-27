import api from "../core/http";
import type { LyricsSourceKind } from "../player/lyrics";

const LYRICS_CACHE_TTL_MS = 30 * 60 * 1000;
const MAX_LYRICS_CACHE_ENTRIES = 256;
const AVAILABILITY_TTL_MS = 10 * 60 * 1000;
const MAX_AVAILABILITY_ENTRIES = 5_000;
const AVAILABILITY_BATCH = 500;
const lyricsCache = new Map<
  string,
  { expiresAt: number; value: LyricsResult }
>();
const lyricsRequests = new Map<string, Promise<LyricsResult>>();
const availabilityCache = new Map<
  string,
  { expiresAt: number; value: LyricsAvailability | null }
>();

export type LyricsResult = {
  id: number;
  trackName: string;
  artistName: string;
  albumName: string;
  duration: number;
  instrumental: boolean;
  plainLyrics: string | null;
  syncedLyrics: string | null;
  /** Where the lyrics came from; older servers leave this out. */
  source?: LyricsSourceKind;
  /** Whether `syncedLyrics` holds timed lines. */
  synced?: boolean;
};

/** Whether a track has lyrics, for the "timed lyrics" marker in lists. */
export type LyricsAvailability = {
  synced: boolean;
  source: LyricsSourceKind;
};

export function getCachedLyrics(songId: string): LyricsResult | undefined {
  const cached = lyricsCache.get(songId);
  if (!cached) return undefined;
  if (cached.expiresAt <= Date.now()) {
    lyricsCache.delete(songId);
    return undefined;
  }

  return cached.value;
}

function rememberAvailability(
  songId: string,
  value: LyricsAvailability | null,
) {
  if (availabilityCache.size >= MAX_AVAILABILITY_ENTRIES) {
    const oldest = availabilityCache.keys().next().value;
    if (oldest !== undefined) availabilityCache.delete(oldest);
  }
  availabilityCache.set(songId, {
    expiresAt: Date.now() + AVAILABILITY_TTL_MS,
    value,
  });
}

export async function findLyrics(songId: string): Promise<LyricsResult> {
  const cached = getCachedLyrics(songId);
  if (cached) return cached;

  const pending = lyricsRequests.get(songId);
  if (pending) return pending;

  const request = api
    .get<LyricsResult>(
      `/lyrics/${encodeURIComponent(songId)}`,
      // Allow LRCLIB more time than the backend's 25-second provider timeout.
      { timeout: 30000 },
    )
    .then((response) => {
      if (lyricsCache.size >= MAX_LYRICS_CACHE_ENTRIES) {
        const oldest = lyricsCache.keys().next().value;
        if (oldest !== undefined) lyricsCache.delete(oldest);
      }
      lyricsCache.set(songId, {
        expiresAt: Date.now() + LYRICS_CACHE_TTL_MS,
        value: response.data,
      });
      const lyrics = response.data;
      if (lyrics?.source && (lyrics.syncedLyrics || lyrics.plainLyrics)) {
        rememberAvailability(songId, {
          source: lyrics.source,
          synced: Boolean(lyrics.synced ?? lyrics.syncedLyrics),
        });
      }
      return response.data;
    })
    .finally(() => lyricsRequests.delete(songId));
  lyricsRequests.set(songId, request);
  return request;
}

/** Availability already known for a song: a value, `null` for none, or
 * `undefined` when it has not been asked. */
export function peekLyricsAvailability(
  songId: string,
): LyricsAvailability | null | undefined {
  const cached = availabilityCache.get(songId);
  if (!cached) return undefined;
  if (cached.expiresAt <= Date.now()) {
    availabilityCache.delete(songId);
    return undefined;
  }
  return cached.value;
}

/**
 * Which of these songs have lyrics and whether they are timed. Reads the
 * files and the server's lyrics cache only; it never asks LRCLIB. Answers
 * are remembered for ten minutes, so lists can ask freely.
 */
export async function getLyricsAvailability(
  songIds: string[],
): Promise<Record<string, LyricsAvailability>> {
  const result: Record<string, LyricsAvailability> = {};
  const missing: string[] = [];
  for (const id of new Set(songIds)) {
    if (!id) continue;
    const known = peekLyricsAvailability(id);
    if (known === undefined) missing.push(id);
    else if (known) result[id] = known;
  }
  for (let start = 0; start < missing.length; start += AVAILABILITY_BATCH) {
    const batch = missing.slice(start, start + AVAILABILITY_BATCH);
    const response = await api.post<{
      songs: Record<string, LyricsAvailability>;
    }>("/lyrics/availability", { song_ids: batch });
    const songs = response.data?.songs ?? {};
    for (const id of batch) {
      const value = songs[id] ?? null;
      rememberAvailability(id, value);
      if (value) result[id] = value;
    }
  }
  return result;
}

let pendingBatch: {
  ids: Set<string>;
  promise: Promise<Record<string, LyricsAvailability>>;
} | null = null;

/**
 * One song's availability. Calls made in the same moment (a list of rows,
 * each asking for itself) share a single batched request.
 */
export function lookupLyricsAvailability(
  songId: string,
): Promise<LyricsAvailability | null> {
  const known = peekLyricsAvailability(songId);
  if (known !== undefined) return Promise.resolve(known);
  if (!pendingBatch) {
    const ids = new Set<string>();
    const promise = new Promise<Record<string, LyricsAvailability>>(
      (resolve, reject) => {
        setTimeout(() => {
          pendingBatch = null;
          getLyricsAvailability([...ids]).then(resolve, reject);
        }, 16);
      },
    );
    pendingBatch = { ids, promise };
  }
  pendingBatch.ids.add(songId);
  return pendingBatch.promise.then((songs) => songs[songId] ?? null);
}

/** Forgets remembered availability (after a rescan, say). */
export function clearLyricsAvailability() {
  availabilityCache.clear();
}
