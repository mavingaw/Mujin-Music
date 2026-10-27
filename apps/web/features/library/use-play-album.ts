"use client";

import { getAlbumInfo, type LibraryAlbum } from "@parson/music-sdk";
import { useCallback } from "react";
import { toast } from "sonner";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";

/** Plays a whole album by id in the selected zone (cards' play button). */
export function usePlayAlbum() {
  const playback = useZonePlayback();
  return useCallback(
    async (albumId: string) => {
      let album: LibraryAlbum;
      try {
        album = (await getAlbumInfo(albumId, false)) as LibraryAlbum;
      } catch {
        toast("Could not load this album.");
        return;
      }
      const artist = album.artist_object;
      if (!album.songs?.length || !artist) return;
      await playback.playQueue(
        album.songs.map((song) => ({ song, album, artist })),
      );
    },
    [playback],
  );
}
