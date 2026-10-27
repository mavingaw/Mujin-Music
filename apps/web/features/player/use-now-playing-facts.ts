"use client";

import { getAlbumInfo, getSongInfo, isStreamingId } from "@parson/music-sdk";
import type { Credit } from "@parson/music-sdk/types";
import { useQuery } from "@tanstack/react-query";
import {
  completeNowPlayingSong,
  nowPlayingFacts,
  type NowPlayingFactsResult,
} from "./now-playing-facts";

interface PlayingSong {
  id?: string;
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
 * The Now Playing facts (album, released, composer, label, track N of M)
 * for whatever is playing, however it started: the song and its album are
 * read from the server, so Next and auto-advance show the same table as a
 * track started from its album page.
 */
export function useNowPlayingFacts(
  song: PlayingSong | null | undefined,
  fallbackAlbum?: { id?: string; name?: string; first_release_date?: string },
  enabled = true,
): NowPlayingFactsResult | undefined {
  const songId = song?.id && !isStreamingId(song.id) ? song.id : null;
  const fullSong = useQuery({
    queryKey: ["now-playing-song", songId],
    queryFn: () => getSongInfo(songId!, false),
    enabled: enabled && Boolean(songId),
    staleTime: 5 * 60_000,
  }).data as PlayingSong | undefined;
  const albumId =
    song?.album_object?.id ||
    fullSong?.album_object?.id ||
    fallbackAlbum?.id ||
    null;
  const fullAlbum = useQuery({
    queryKey: ["now-playing-album", albumId],
    queryFn: () => getAlbumInfo(albumId!, false),
    enabled: enabled && Boolean(albumId) && !isStreamingId(albumId),
    staleTime: 5 * 60_000,
  }).data as
    | {
        id?: string;
        name?: string;
        first_release_date?: string;
        songs?: unknown[];
        label?: string | null;
      }
    | undefined;
  if (!enabled || !song) return undefined;
  return nowPlayingFacts(
    completeNowPlayingSong(song, fullSong, fullAlbum),
    fallbackAlbum,
  );
}
