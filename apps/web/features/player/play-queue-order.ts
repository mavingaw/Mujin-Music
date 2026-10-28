import type { Artist, LibraryAlbum } from "@parson/music-sdk";
import type { QueueInput } from "./player-model";

/** An album's tracks as queue items, in album order. */
export function albumQueue(album: LibraryAlbum): QueueInput[] {
  const artist = album.artist_object;
  if (!artist) return [];
  return (album.songs ?? []).map((song) => ({ song, album, artist }));
}

/**
 * An artist's tracks as queue items: albums oldest first (undated last, in
 * catalogue order), each track once even when it is on two releases.
 */
export function artistQueue(artist: Artist): QueueInput[] {
  const seen = new Set<string>();
  const albums = artist.albums
    .map((album, index) => ({ album, index }))
    .sort((left, right) => {
      const a = left.album.first_release_date || "";
      const b = right.album.first_release_date || "";
      if (a && b && a !== b) return a.localeCompare(b);
      if (Boolean(a) !== Boolean(b)) return a ? -1 : 1;
      return left.index - right.index;
    })
    .map(({ album }) => album);
  return albums.flatMap((album) =>
    album.songs
      .filter((song) => {
        if (!song.id || seen.has(song.id)) return false;
        seen.add(song.id);
        return true;
      })
      .map((song) => ({ song, album, artist })),
  );
}
