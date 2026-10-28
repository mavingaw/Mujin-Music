import type { QueueInput } from "@/features/player/player-model";
import {
  formatLabel,
  sampleRateLabel,
  type AudioProperties,
  type CompositionRecording,
  type FocusQuery,
  type FocusSong,
  type LibraryCatalogSong,
} from "@parson/music-sdk";

export {
  entityNoun,
  focusCountLabel,
  focusedAlbumTracks,
  usesFocusEndpoint,
} from "@parson/music-sdk";

/** A compact technical label such as "FLAC 24/96" or "MP3 320". */
export function audioBadge(
  audio: AudioProperties | null | undefined,
): string | null {
  if (!audio) return null;
  // Generated treble in a FLAC: name what it is, not the container.
  if (audio.enhanced_by) return "AI-enhanced";
  const codec =
    audio.codec === "pcm" || audio.codec === "pcm_float"
      ? (audio.container ?? audio.codec)
      : audio.codec;
  if (!codec) return null;
  const name = formatLabel(codec);
  if (audio.lossless) {
    const depth = audio.bit_depth ? `${audio.bit_depth}` : null;
    const rate = audio.sample_rate
      ? sampleRateLabel(audio.sample_rate).replace(" kHz", "")
      : null;
    if (depth && rate) return `${name} ${depth}/${rate}`;
    return name;
  }
  return audio.bitrate_kbps ? `${name} ${audio.bitrate_kbps}` : name;
}

export function qualityBadgeLabel(
  quality: FocusSong["quality"] | undefined,
): string | null {
  switch (quality) {
    case "hires":
      return "Hi-Res";
    case "cd":
      return "Lossless";
    default:
      return null;
  }
}

/** Query key fragment that ignores fields that do not change results. */
export function focusQueryKey(query: FocusQuery) {
  return {
    entity: query.entity,
    match: query.match ?? "all",
    filters: query.filters,
    text: query.text?.trim() || null,
    sort: query.sort ?? null,
    direction: query.direction ?? null,
    seed: query.seed ?? null,
    scope: query.scope ?? [],
  };
}

/** Facet counts do not depend on sort order. */
export function facetQueryKey(query: FocusQuery) {
  return {
    entity: query.entity,
    match: query.match ?? "all",
    filters: query.filters,
    text: query.text?.trim() || null,
    scope: query.scope ?? [],
  };
}

/** "4 recordings · 3 movements" style summary for a composition. */
export function compositionSummary(composition: {
  movementCount: number;
  recordings: unknown[];
}): string {
  const parts = [
    `${composition.recordings.length} ${composition.recordings.length === 1 ? "recording" : "recordings"}`,
  ];
  if (composition.movementCount > 1) {
    parts.push(`${composition.movementCount} movements`);
  }
  return parts.join(" · ");
}

/** Player queue entries for catalog-shaped songs. */
export function catalogSongsToQueue(songs: LibraryCatalogSong[]): QueueInput[] {
  return songs.map((song) => ({
    song: {
      id: song.id,
      name: song.name,
      artist: song.artistName,
      path: song.path,
      duration: song.durationSeconds,
    },
    artist: { id: song.artistId, name: song.artistName },
    album: {
      id: song.albumId,
      name: song.albumName,
      cover_url: song.coverPath,
    },
  }));
}

/** Fisher-Yates shuffle with an injectable random source for tests. */
export function shuffled<T>(
  items: T[],
  random: () => number = Math.random,
): T[] {
  const copy = [...items];
  for (let index = copy.length - 1; index > 0; index -= 1) {
    const swap = Math.floor(random() * (index + 1));
    [copy[index], copy[swap]] = [copy[swap]!, copy[index]!];
  }
  return copy;
}

/** Milliseconds since a server UTC timestamp ("YYYY-MM-DD HH:MM:SS"). */
export function smartRefreshAge(refreshedAt: string, now: number): number {
  const parsed = Date.parse(
    /[zZ]|[+-]\d\d:?\d\d$/.test(refreshedAt)
      ? refreshedAt
      : `${refreshedAt.replace(" ", "T")}Z`,
  );
  return Number.isFinite(parsed) ? Math.max(0, now - parsed) : Infinity;
}

export function smartRefreshLabel(refreshedAt: string, now: number): string {
  const age = smartRefreshAge(refreshedAt, now);
  if (!Number.isFinite(age)) return "Not refreshed yet";
  const minutes = Math.floor(age / 60_000);
  if (minutes < 1) return "Updated just now";
  if (minutes < 60) return `Updated ${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Updated ${hours} h ago`;
  const days = Math.floor(hours / 24);
  return `Updated ${days} day${days === 1 ? "" : "s"} ago`;
}

/** Queue entries for one recording of a composition, in movement order. */
export function recordingQueue(recording: CompositionRecording): QueueInput[] {
  const artist = {
    id: recording.artistId ?? "",
    name: recording.artistName ?? "",
  };
  const album = {
    id: recording.albumId ?? "",
    name: recording.albumName ?? "",
    cover_url: recording.coverPath ?? "",
  };
  return recording.tracks.map((track) => ({
    song: {
      id: track.id,
      name: track.title,
      artist: artist.name,
      duration: track.durationSeconds,
    },
    artist,
    album,
  }));
}
