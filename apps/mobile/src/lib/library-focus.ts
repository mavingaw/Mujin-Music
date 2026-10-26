import {
  focusSummary,
  workDescription,
  type CompositionRecording,
  type FilterState,
  type FocusWork,
  type LibrarySong,
  type FocusEntity,
  type FocusQuery,
} from "@parson/music-sdk";

import { playableCatalogSong } from "@/lib/playable-song";

export type LibrarySection =
  | "albums"
  | "songs"
  | "downloads"
  | "liked"
  | "artists"
  | "composers"
  | "compositions"
  | "playlists"
  | "banned";

/** The browser a library section focuses, or null when it has no Focus. */
export function focusEntityForSection(
  section: LibrarySection,
): FocusEntity | null {
  return section === "albums" ||
    section === "songs" ||
    section === "artists" ||
    section === "composers" ||
    section === "compositions"
    ? section
    : null;
}

/** "Beethoven · Symphony for Orchestra · Composed 1812" under a work. */
export function workCaption(
  work: Pick<
    FocusWork,
    | "composers"
    | "form"
    | "instrumentation"
    | "scoring"
    | "composedYear"
    | "period"
  >,
): string {
  return [
    work.composers.map((composer) => composer.name).join(", "),
    workDescription(work),
  ]
    .filter(Boolean)
    .join(" · ");
}

/** "4:05" for a duration in seconds; empty when unknown. */
export function durationLabel(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "";
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  return `${minutes}:${rest < 10 ? "0" : ""}${rest}`;
}

/** A recording's tracks as playable songs, in movement order. */
export function recordingSongs(recording: CompositionRecording): LibrarySong[] {
  return recording.tracks.map((track) =>
    playableCatalogSong({
      id: track.id,
      name: track.title,
      artistId: recording.artistId ?? undefined,
      artistName: recording.artistName ?? "",
      albumId: recording.albumId ?? undefined,
      albumName: recording.albumName ?? "",
      coverPath: recording.coverPath ?? undefined,
      duration: track.durationSeconds,
    }),
  );
}

/** "Claudio Abbado, Berliner Philharmoniker · 1978" under a recording. */
export function recordingCaption(recording: CompositionRecording): string {
  return [
    recording.performers.length
      ? recording.performers.join(", ")
      : recording.artistName,
    recording.releaseYear,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** Default name for a smart playlist made from a focus. */
export function smartPlaylistName(query: FocusQuery): string {
  const summary = focusSummary(query).trim();
  if (!summary) return "Smart playlist";
  return summary.length > 200 ? `${summary.slice(0, 199)}…` : summary;
}

export type ChipTone = "include" | "exclude" | "idle";

export function chipTone(state: FilterState): ChipTone {
  return state ?? "idle";
}

/** Spoken description for a facet value chip. */
export function facetChipAccessibilityLabel(
  label: string,
  count: number,
  state: FilterState,
): string {
  const noun = count === 1 ? "item" : "items";
  if (state === "include") {
    return `${label}, ${count} ${noun}, focused. Double tap to exclude.`;
  }
  if (state === "exclude") {
    return `${label}, excluded. Double tap to clear.`;
  }
  return `${label}, ${count} ${noun}. Double tap to focus.`;
}
