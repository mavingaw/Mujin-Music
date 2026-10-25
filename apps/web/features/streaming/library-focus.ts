import type {
  CatalogAlbum,
  CatalogFacet,
  CatalogFilter,
  CatalogQuality,
  CatalogSort,
} from "@parson/music-sdk";
import {
  parseAlbumSourceFilter,
  type AlbumSourceFilter,
  type LibraryAlbumEntry,
} from "./streaming-model";

/**
 * Roon-style Focus for the Library's albums: storage location (source),
 * genre, decade and quality, combined. Kept in the URL (`?source=&genre=
 * &decade=&quality=`) so a focused view can be shared or reloaded.
 */
export type LibraryFocus = {
  source: AlbumSourceFilter;
  genre: string | null;
  decade: number | null;
  quality: CatalogQuality | null;
};

export const emptyFocus: LibraryFocus = {
  source: "all",
  genre: null,
  decade: null,
  quality: null,
};

const qualities = ["hi_res", "lossless", "lossy"] as const;

export function parseLibraryFocus(params: URLSearchParams): LibraryFocus {
  const decade = Number(params.get("decade"));
  const genre = params.get("genre")?.trim();
  const quality = params.get("quality");
  return {
    source: parseAlbumSourceFilter(params.get("source")),
    genre: genre ? genre.slice(0, 128) : null,
    decade:
      Number.isInteger(decade) &&
      decade >= 1000 &&
      decade <= 2990 &&
      decade % 10 === 0
        ? decade
        : null,
    quality: qualities.includes(quality as CatalogQuality)
      ? (quality as CatalogQuality)
      : null,
  };
}

/** Writes the focus into `params`, leaving other parameters alone. */
export function writeLibraryFocus(
  params: URLSearchParams,
  focus: LibraryFocus,
): URLSearchParams {
  const next = new URLSearchParams(params);
  const set = (key: string, value: string | null) => {
    if (value === null) next.delete(key);
    else next.set(key, value);
  };
  set("source", focus.source === "all" ? null : focus.source);
  set("genre", focus.genre);
  set("decade", focus.decade === null ? null : String(focus.decade));
  set("quality", focus.quality);
  return next;
}

export function toCatalogFilter(focus: LibraryFocus): CatalogFilter {
  return {
    ...(focus.source !== "all" ? { source: focus.source } : {}),
    ...(focus.genre ? { genre: focus.genre } : {}),
    ...(focus.decade !== null ? { decade: focus.decade } : {}),
    ...(focus.quality ? { quality: focus.quality } : {}),
  };
}

/** How many filters are on, beyond "All sources". */
export function activeFocusCount(focus: LibraryFocus): number {
  return (
    Number(focus.source !== "all") +
    Number(focus.genre !== null) +
    Number(focus.decade !== null) +
    Number(focus.quality !== null)
  );
}

/** `1990` -> `1990-1999`, as Roon labels decades. */
export function decadeLabel(decade: number): string {
  return `${decade}-${decade + 9}`;
}

export const qualityLabels: Record<CatalogQuality, string> = {
  hi_res: "Hi-Res",
  lossless: "CD quality",
  lossy: "Lossy",
};

/** Plain-language summary, e.g. "Jazz, 1990-1999 in Qobuz library". */
export function describeFocus(
  focus: LibraryFocus,
  sourceNames: Record<string, string>,
): string | null {
  const parts = [
    focus.genre,
    focus.decade === null ? null : decadeLabel(focus.decade),
    focus.quality ? qualityLabels[focus.quality] : null,
  ].filter(Boolean);
  const where =
    focus.source === "all"
      ? null
      : focus.source === "local"
        ? "local files"
        : `${sourceNames[focus.source] ?? focus.source} library`;
  if (!parts.length && !where) return null;
  return [parts.join(", "), where ? `in ${where}` : null]
    .filter(Boolean)
    .join(" ");
}

/**
 * The facet values worth showing as chips: the most common ones up to
 * `max`, plus the selected value wherever it ranks. The rest go in a menu.
 */
export function splitFacets<T>(
  facets: CatalogFacet<T>[],
  selected: T | null,
  max: number,
): { shown: CatalogFacet<T>[]; more: CatalogFacet<T>[] } {
  const shown = facets.slice(0, max);
  const more = facets.slice(max);
  const index = more.findIndex((facet) => facet.value === selected);
  if (index >= 0) {
    shown.push(...more.splice(index, 1));
  } else if (
    selected !== null &&
    !shown.some((facet) => facet.value === selected)
  ) {
    // Selected but matching nothing now: keep it visible so it can be cleared.
    shown.push({ value: selected, count: 0 });
  }
  return { shown, more };
}

export function catalogAlbumEntry(album: CatalogAlbum): LibraryAlbumEntry {
  return {
    key: album.key,
    id: album.id,
    name: album.name,
    artistId: album.artist_id,
    artistName: album.artist_name,
    cover: album.cover,
    year: album.year ? String(album.year) : "",
    source: album.source === "local" ? null : album.source,
    formatLabel: album.format_label,
  };
}

/** Album orders, in the menu's order. Artist is the default. */
export const catalogSorts: { value: CatalogSort; label: string }[] = [
  { value: "artist", label: "Artist" },
  { value: "title", label: "Title" },
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "quality", label: "Best quality first" },
];

/** The album order from `?sort=`; anything unknown is the default. */
export function parseCatalogSort(params: URLSearchParams): CatalogSort {
  const value = params.get("sort");
  return catalogSorts.some((sort) => sort.value === value)
    ? (value as CatalogSort)
    : "artist";
}

/** Writes the album order into `params`; the default is left out. */
export function writeCatalogSort(
  params: URLSearchParams,
  sort: CatalogSort,
): URLSearchParams {
  const next = new URLSearchParams(params);
  if (sort === "artist") next.delete("sort");
  else next.set("sort", sort);
  return next;
}
