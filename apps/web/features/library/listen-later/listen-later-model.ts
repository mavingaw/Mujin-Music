import type { ListenLaterCounts, ListenLaterItem } from "@parson/music-sdk";

const plural = (count: number, one: string, many: string) =>
  `${count} ${count === 1 ? one : many}`;

/** Header line: "12 albums · 4 tracks · 2 artists · 140 songs to hear". */
export function listenLaterSummary(
  counts: ListenLaterCounts,
  items: ListenLaterItem[],
): string {
  if (!counts.all) return "Music you set aside, in one place.";
  const parts = [
    counts.album ? plural(counts.album, "album", "albums") : null,
    counts.track ? plural(counts.track, "track", "tracks") : null,
    counts.artist ? plural(counts.artist, "artist", "artists") : null,
  ].filter(Boolean);
  const songs = items.reduce(
    (total, item) =>
      total + (item.album ? item.album.songCount : item.song ? 1 : 0),
    0,
  );
  if (songs > 0 && (counts.album > 0 || songs !== counts.track)) {
    parts.push(`${plural(songs, "song", "songs")} to hear`);
  }
  return parts.join(" · ");
}
