import api, { isApiError } from "../core/http";
import type {
  AlbumInfo,
  ArtistInfo,
  AudioProperties,
  Loudness,
} from "../domain/types";

export interface PlaybackQueueSong {
  id: string;
  name: string;
  artist: string;
  contributing_artists: string[];
  contributing_artist_ids: string[];
  track_number: number;
  duration: number;
  album_object: AlbumInfo;
  artist_object: ArtistInfo;
  origin: "manual" | "generated";
  queue_position: number;
  /** Omitted when the server has not read the file's stream headers. */
  audio?: AudioProperties;
  /** ReplayGain tags; omitted when the file has none. */
  loudness?: Loudness;
}

/** What a Radio station is built around. */
export type RadioSeedKind = "song" | "album" | "artist" | "genre";

export interface RadioSeed {
  kind: RadioSeedKind;
  /** Catalog id, or the genre name for genre stations. */
  id: string;
}

/** A seed as the server resolved it, with its current display name. */
export interface ResolvedRadioSeed extends RadioSeed {
  name: string;
}

export interface PlaybackQueue {
  id: string;
  revision: number;
  current_position: number;
  /** Why the queue exists, e.g. `radio` or `manual_selection`. */
  source?: string;
  /** What a Radio queue was built around; absent for profile-only Radio. */
  seed?: ResolvedRadioSeed;
  items: PlaybackQueueSong[];
}

export interface RadioSeedParams {
  seed_song_id?: string;
  seed_album_id?: string;
  seed_artist_id?: string;
  seed_genre?: string;
}

/** Maps a seed to the request fields the queue and picks endpoints take. */
export function radioSeedParams(seed?: RadioSeed | null): RadioSeedParams {
  if (!seed?.id) return {};
  switch (seed.kind) {
    case "song":
      return { seed_song_id: seed.id };
    case "album":
      return { seed_album_id: seed.id };
    case "artist":
      return { seed_artist_id: seed.id };
    case "genre":
      return { seed_genre: seed.id };
  }
}

export interface PlaybackQueueRevisionConflict {
  revision: number;
  current_position: number;
}

export function getPlaybackQueueRevisionConflict(
  error: unknown,
): PlaybackQueueRevisionConflict | null {
  if (!isApiError(error) || error.response?.status !== 409) return null;
  const data = error.response.data as Partial<
    PlaybackQueueRevisionConflict & { error: string }
  >;
  return data?.error === "queue_revision_conflict" &&
    Number.isInteger(data.revision) &&
    Number(data.revision) >= 1 &&
    Number.isInteger(data.current_position) &&
    Number(data.current_position) >= 0
    ? {
        revision: Number(data.revision),
        current_position: Number(data.current_position),
      }
    : null;
}

export async function createPlaybackQueue(
  options: RadioSeedParams & {
    explicit_song_ids?: string[];
    /**
     * The queue so far, oldest first. Radio avoids these while fresh songs
     * remain, then lets them back in, least recently played first.
     */
    exclude_song_ids?: string[];
    /** Songs Radio must never pick (thumbs down this session). */
    reject_song_ids?: string[];
    generated_items?: number;
    source?: string;
  },
): Promise<PlaybackQueue> {
  const response = await api.post<PlaybackQueue>("/playback/queues", options);
  return response.data;
}

export async function getPlaybackQueue(id: string): Promise<PlaybackQueue> {
  const response = await api.get<PlaybackQueue>(
    `/playback/queues/${encodeURIComponent(id)}`,
  );
  return response.data;
}

export async function updatePlaybackQueuePosition(
  id: string,
  currentPosition: number,
  revision: number,
): Promise<{ current_position: number; revision: number }> {
  const response = await api.patch<{
    current_position: number;
    revision: number;
  }>(`/playback/queues/${encodeURIComponent(id)}`, {
    current_position: currentPosition,
    revision,
  });
  return response.data;
}

export interface RadioPick extends Omit<PlaybackQueueSong, "origin"> {
  origin: "generated";
  /** Short, listener-facing reason, e.g. "from this artist". */
  reason: string;
}

export interface RadioPicks {
  seed?: ResolvedRadioSeed;
  items: RadioPick[];
  /**
   * True when Radio has nothing left to play: every song in the library is
   * banned or thumbed down. Radio recycles played songs, so this is the only
   * way it runs out.
   */
  exhausted?: boolean;
  message?: string;
}

export const radioExhaustedMessage =
  "Radio has nothing left to play: every track in your library is banned or thumbed down.";

/** The listener-facing message when Radio cannot continue, otherwise null. */
export function radioExhaustedNotice(error: unknown): string | null {
  if (!isApiError(error) || error.response?.status !== 422) return null;
  const data = error.response.data as
    { error?: string; message?: string } | undefined;
  if (data?.error !== "radio_exhausted") return null;
  return data.message || radioExhaustedMessage;
}

/** The message for a picks response that came back exhausted, else null. */
export function radioPicksExhaustedNotice(
  picks: Pick<RadioPicks, "exhausted" | "message" | "items">,
): string | null {
  return picks.exhausted && !picks.items.length
    ? picks.message || radioExhaustedMessage
    : null;
}

/**
 * Fresh Radio picks that do not create a queue. Used to keep the Radio
 * section of a queue topped up and to replace a thumbed-down pick.
 */
export async function getRadioPicks(options: {
  seed?: RadioSeed | null;
  /** The queue so far, oldest first (see {@link createPlaybackQueue}). */
  exclude_song_ids?: string[];
  /** Songs Radio must never pick. */
  reject_song_ids?: string[];
  limit?: number;
}): Promise<RadioPicks> {
  const response = await api.post<RadioPicks>("/playback/radio/picks", {
    ...radioSeedParams(options.seed),
    exclude_song_ids: (options.exclude_song_ids ?? []).slice(-500),
    ...(options.reject_song_ids?.length
      ? { reject_song_ids: options.reject_song_ids.slice(-500) }
      : {}),
    limit: options.limit,
  });
  return response.data;
}

export interface RadioSettings {
  /** Continue with Radio picks once the queue runs out. */
  enabled: boolean;
  /** Show a "Coming up" notice with thumbs before a Radio pick plays. */
  coming_up_notice: boolean;
}

export const defaultRadioSettings: RadioSettings = {
  enabled: true,
  coming_up_notice: true,
};

export async function getRadioSettings(): Promise<RadioSettings> {
  const response = await api.get<RadioSettings>("/playback/radio/settings", {
    timeout: 4_000,
  });
  return response.data;
}

export async function updateRadioSettings(
  update: Partial<RadioSettings>,
): Promise<RadioSettings> {
  const response = await api.put<RadioSettings>(
    "/playback/radio/settings",
    update,
  );
  return response.data;
}
