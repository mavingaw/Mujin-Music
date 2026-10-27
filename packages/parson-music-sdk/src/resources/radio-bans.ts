import api from "../core/http";
import type { PlaybackQueueSong } from "./playback";

/** What a ban applies to. Banning an album or artist covers its tracks. */
export type RadioBanKind = "song" | "album" | "artist";

export interface RadioBanTarget {
  kind: RadioBanKind;
  id: string;
}

/**
 * A stored ban. `name` is null when the banned item has left the library;
 * the ban stays so it applies again if the item comes back.
 */
export interface RadioBan extends RadioBanTarget {
  name: string | null;
  /** Artist (and album, for tracks) for display. */
  subtitle?: string;
  image_url?: string;
  artist_id?: string;
  album_id?: string;
  /** Playable track, for track bans still in the library. */
  song?: PlaybackQueueSong;
  created_at: string;
}

/** A track Radio keeps out after a thumbs down or a run of early skips. */
export interface RadioDislike {
  song: PlaybackQueueSong;
  disliked_at?: string;
}

/** Every ban the signed-in listener holds, newest first. */
export async function getRadioBans(): Promise<RadioBan[]> {
  const response = await api.get<{ items: RadioBan[] }>("/playback/radio/bans");
  return response.data.items;
}

/**
 * Bans a track, album or artist from Radio, Shuffle and recommendations.
 * Applies to the very next pick. Explicit plays still work.
 */
export async function banFromRadio(target: RadioBanTarget): Promise<RadioBan> {
  const response = await api.post<RadioBan>("/playback/radio/bans", {
    kind: target.kind,
    id: target.id,
  });
  return response.data;
}

export async function unbanFromRadio(target: RadioBanTarget): Promise<void> {
  await api.delete(
    `/playback/radio/bans/${encodeURIComponent(target.kind)}/${encodeURIComponent(target.id)}`,
  );
}

/** Tracks Radio keeps out because the listener disliked them. */
export async function getRadioDislikes(): Promise<RadioDislike[]> {
  const response = await api.get<{ items: RadioDislike[] }>(
    "/playback/radio/dislikes",
  );
  return response.data.items;
}

/** Undoes one dislike: the track can come back through Radio. */
export async function clearRadioDislike(songId: string): Promise<void> {
  await api.delete(`/playback/radio/dislikes/${encodeURIComponent(songId)}`);
}
