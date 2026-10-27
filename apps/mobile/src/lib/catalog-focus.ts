import type {
  CatalogFacets,
  CatalogFilter,
  CatalogSource,
} from "@parson/music-sdk";

/** The Library's album focus: source, genre and decade, combined. */
export type MobileFocus = {
  source: CatalogSource | null;
  genre: string | null;
  decade: number | null;
};

export const noFocus: MobileFocus = { source: null, genre: null, decade: null };

export function focusFilter(focus: MobileFocus): CatalogFilter {
  return {
    ...(focus.source ? { source: focus.source } : {}),
    ...(focus.genre ? { genre: focus.genre } : {}),
    ...(focus.decade !== null ? { decade: focus.decade } : {}),
  };
}

export type FocusChip = {
  key: string;
  label: string;
  count: number;
  active: boolean;
  /** The focus after tapping this chip (tapping an active chip clears it). */
  next: MobileFocus;
};

const SOURCE_LABELS: Record<CatalogSource, string> = {
  local: "Local files",
  tidal: "TIDAL library",
  qobuz: "Qobuz library",
};

/**
 * One scrolling row of chips: sources, then the most common genres, then
 * decades. Chips that would show nothing are left out unless selected.
 */
export function focusChips(
  facets: CatalogFacets | undefined,
  focus: MobileFocus,
  maxGenres = 6,
): FocusChip[] {
  if (!facets) return [];
  const sources = facets.sources.filter(
    (facet) => facet.count > 0 || facet.value === focus.source,
  );
  const chips: FocusChip[] =
    sources.length > 1 || focus.source
      ? sources.map((facet) => ({
          key: `source:${facet.value}`,
          label: SOURCE_LABELS[facet.value],
          count: facet.count,
          active: focus.source === facet.value,
          next: {
            ...focus,
            source: focus.source === facet.value ? null : facet.value,
          },
        }))
      : [];
  const genres = facets.genres.slice(0, maxGenres);
  if (focus.genre && !genres.some((facet) => facet.value === focus.genre)) {
    genres.push(
      facets.genres.find((facet) => facet.value === focus.genre) ?? {
        value: focus.genre,
        count: 0,
      },
    );
  }
  for (const facet of genres) {
    chips.push({
      key: `genre:${facet.value}`,
      label: facet.value,
      count: facet.count,
      active: focus.genre === facet.value,
      next: {
        ...focus,
        genre: focus.genre === facet.value ? null : facet.value,
      },
    });
  }
  for (const facet of facets.decades) {
    chips.push({
      key: `decade:${facet.value}`,
      label: `${facet.value}s`,
      count: facet.count,
      active: focus.decade === facet.value,
      next: {
        ...focus,
        decade: focus.decade === facet.value ? null : facet.value,
      },
    });
  }
  // Active chips first, so a focused view shows what is on.
  return [
    ...chips.filter((chip) => chip.active),
    ...chips.filter((chip) => !chip.active),
  ];
}
