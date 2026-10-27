/**
 * The word above an artist's name. Someone with albums of their own is an
 * artist (or a group, or an orchestra) first, even when they also wrote
 * every song; "Composer" is for people the library knows only from
 * composer credits, and "Contributor" for other credited people.
 */
export function artistKicker({
  ownsAlbums,
  isComposer,
  inLibrary,
  kind,
}: {
  ownsAlbums: boolean;
  isComposer: boolean;
  inLibrary: boolean;
  kind?: string | null;
}): string {
  const group = kind === "Group" || kind === "Orchestra" ? kind : null;
  if (ownsAlbums) return group ?? "Artist";
  if (isComposer) return "Composer";
  if (!inLibrary) return "Contributor";
  return group ?? "Artist";
}

/** "2 albums · 8 tracks" for the Main artist chip. */
export function mainArtistDetail(work: { albums: number; tracks: number }) {
  const albums = `${work.albums} ${work.albums === 1 ? "album" : "albums"}`;
  return work.tracks > 0
    ? `${albums} · ${work.tracks} ${work.tracks === 1 ? "track" : "tracks"}`
    : albums;
}
