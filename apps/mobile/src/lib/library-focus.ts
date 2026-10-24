import {
  focusSummary,
  type FilterState,
  type FocusEntity,
  type FocusQuery,
} from "@parson/music-sdk";

export type LibrarySection =
  "albums" | "songs" | "downloads" | "liked" | "artists" | "playlists";

/** The browser a library section focuses, or null when it has no Focus. */
export function focusEntityForSection(
  section: LibrarySection,
): FocusEntity | null {
  return section === "albums" || section === "songs" || section === "artists"
    ? section
    : null;
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
