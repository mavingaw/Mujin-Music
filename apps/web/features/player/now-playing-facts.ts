import type { Credit } from "@parson/music-sdk/types";

/** The metadata stack shown under the title in Now Playing. */
export interface NowPlayingFactsResult {
  albumId?: string;
  albumName?: string;
  year?: string;
  composers: string[];
  label?: string | null;
  trackNumber?: number;
  trackCount?: number;
}

interface SongLike {
  track_number?: number;
  label?: string | null;
  credits?: Credit[];
  album_object: {
    id?: string;
    name?: string;
    first_release_date?: string;
    songs?: unknown[];
  };
}

/**
 * Album, release year, composers (deduplicated, in credit order), label and
 * track position for the current song, falling back to the player's album
 * when the song carries none.
 */
export function nowPlayingFacts(
  song: SongLike,
  fallbackAlbum?: { id?: string; name?: string; first_release_date?: string },
): NowPlayingFactsResult {
  const album = song.album_object;
  const date = album.first_release_date || fallbackAlbum?.first_release_date;
  const year = /^\d{4}/.exec(date ?? "")?.[0];
  const composers: string[] = [];
  for (const credit of song.credits ?? []) {
    if (credit.role !== "composer") continue;
    const name = credit.name.trim();
    if (name && !composers.includes(name)) composers.push(name);
  }
  const trackCount = album.songs?.length;
  return {
    albumId: album.id || fallbackAlbum?.id || undefined,
    albumName: album.name || fallbackAlbum?.name || undefined,
    year: year && year !== "0000" ? year : undefined,
    composers,
    label: song.label?.trim() || null,
    trackNumber: song.track_number || undefined,
    trackCount: trackCount && trackCount > 1 ? trackCount : undefined,
  };
}
