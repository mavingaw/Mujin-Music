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

interface AlbumLike {
  id?: string;
  name?: string;
  first_release_date?: string;
  songs?: unknown[];
  label?: string | null;
}

/**
 * Fills what the playing song lacks from the song and album as the server
 * knows them. A track reached by Next or by auto-advance often comes from a
 * lean queue entry (no credits, no label, no track list), and the facts
 * table must not shrink because of how the track was started.
 */
export function completeNowPlayingSong<T extends SongLike>(
  playing: T,
  fetchedSong?: SongLike | null,
  fetchedAlbum?: AlbumLike | null,
): SongLike {
  const album = playing.album_object;
  const fetchedSongAlbum = fetchedSong?.album_object;
  const sameAlbum = (candidate?: { id?: string } | null) =>
    !candidate?.id || !album.id || candidate.id === album.id;
  const albumInfo = sameAlbum(fetchedAlbum) ? fetchedAlbum : null;
  const songAlbum = sameAlbum(fetchedSongAlbum) ? fetchedSongAlbum : null;
  const songs = [album.songs, albumInfo?.songs, songAlbum?.songs].find((list) =>
    Boolean(list?.length),
  );
  return {
    track_number: playing.track_number || fetchedSong?.track_number,
    label:
      playing.label?.trim() ||
      fetchedSong?.label?.trim() ||
      albumInfo?.label?.trim() ||
      null,
    credits: playing.credits?.length ? playing.credits : fetchedSong?.credits,
    album_object: {
      id: album.id || songAlbum?.id || albumInfo?.id,
      name: album.name || songAlbum?.name || albumInfo?.name,
      first_release_date:
        validDate(album.first_release_date) ||
        validDate(songAlbum?.first_release_date) ||
        validDate(albumInfo?.first_release_date) ||
        album.first_release_date,
      songs,
    },
  };
}

function validDate(date?: string) {
  return date && /^\d{4}/.test(date) && !date.startsWith("0000")
    ? date
    : undefined;
}
