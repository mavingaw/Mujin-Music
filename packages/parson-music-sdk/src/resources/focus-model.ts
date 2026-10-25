/**
 * Pure helpers shared by the web and mobile Focus UIs: filter identity,
 * Roon-style include/exclude cycling, chip labels, facet titles, sort
 * options and URL encoding.
 */
import type {
  Facet,
  FacetKey,
  FacetValue,
  FocusCriterion,
  FocusEntity,
  FocusFilter,
  FocusQuery,
  FocusSort,
  QualityTier,
  WorkFacts,
} from "./focus";

export type FilterState = "include" | "exclude" | null;

const RANGE_FACETS = new Set(["year", "composed", "sample_rate", "bit_depth"]);

export function emptyFocus(entity: FocusEntity = "albums"): FocusQuery {
  return { entity, filters: [] };
}

/** A focus over a fixed subset, e.g. a composer's works or an album. */
export function scopedFocus(
  entity: FocusEntity,
  scope: FocusCriterion[],
): FocusQuery {
  return { entity, filters: [], scope };
}

/** The scope criterion for one composer's works. */
export function composerScope(
  artistId: string,
  name?: string | null,
): FocusCriterion {
  return { facet: "credit", role: "composer", artist_id: artistId, name };
}

/** The scope criterion for one work's recordings. */
export function workScope(
  workId: string,
  name?: string | null,
): FocusCriterion {
  return { facet: "work", work_id: workId, name };
}

/** The scope criterion for one album's tracks. */
export function albumScope(
  albumId: string,
  name?: string | null,
): FocusCriterion {
  return { facet: "album", album_id: albumId, name };
}

/** Clears filters and text while keeping the browser, sort and scope. */
export function clearedFocus(query: FocusQuery): FocusQuery {
  return { ...query, filters: [], text: null, match: undefined };
}

export function normalizedKey(value: string): string {
  return value.trim().toLowerCase();
}

/** Identity of a criterion: two filters with the same key are one chip. */
export function filterKey(criterion: FocusCriterion): string {
  switch (criterion.facet) {
    case "genre":
    case "format":
    case "label":
    case "release_type":
    case "form":
    case "instrumentation":
    case "period":
    case "nationality":
      return `${criterion.facet}:${normalizedKey(criterion.value)}`;
    case "composed":
      return "composed";
    case "work":
      return `work:${criterion.work_id}`;
    case "quality":
      return `quality:${criterion.value}`;
    case "year":
    case "sample_rate":
    case "bit_depth":
      return criterion.facet;
    case "credit":
      return `credit:${criterion.role}:${criterion.artist_id}`;
    case "artist":
      return `artist:${criterion.artist_id}`;
    case "album":
      return `album:${criterion.album_id}`;
    case "tag":
      return `tag:${criterion.tag_id}`;
    case "source":
      return `source:${criterion.root_id}`;
    case "added_within":
      return "added_within";
    case "liked":
    case "played":
      return criterion.facet;
  }
}

function withoutExclude(filter: FocusFilter): FocusCriterion {
  const { exclude: _exclude, ...criterion } = filter;
  return criterion as FocusCriterion;
}

export function filterState(
  query: FocusQuery,
  criterion: FocusCriterion,
): FilterState {
  const key = filterKey(criterion);
  const match = query.filters.find((filter) => filterKey(filter) === key);
  if (!match) return null;
  if (RANGE_FACETS.has(criterion.facet)) {
    // A range chip is "on" for a value only when it is that exact range.
    const same =
      JSON.stringify(rangeOf(withoutExclude(match))) ===
      JSON.stringify(rangeOf(criterion));
    if (!same) return null;
  }
  return match.exclude ? "exclude" : "include";
}

function rangeOf(criterion: FocusCriterion): [number | null, number | null] {
  switch (criterion.facet) {
    case "year":
    case "composed":
      return [criterion.from ?? null, criterion.to ?? null];
    case "sample_rate":
    case "bit_depth":
      return [criterion.min ?? null, criterion.max ?? null];
    default:
      return [null, null];
  }
}

function replaceFilter(
  query: FocusQuery,
  key: string,
  next: FocusFilter | null,
): FocusQuery {
  const index = query.filters.findIndex((filter) => filterKey(filter) === key);
  const filters = [...query.filters];
  if (index === -1) {
    if (next) filters.push(next);
  } else if (next) {
    filters[index] = next;
  } else {
    filters.splice(index, 1);
  }
  return { ...query, filters };
}

/**
 * Applies a click on a facet value the way Roon does: the first click
 * focuses on it, the second inverts it into an exclusion and the third
 * clears it. Range facets (year, sample rate, bit depth) hold one range, so
 * choosing a different range replaces the current one.
 */
export function cycleFilter(
  query: FocusQuery,
  criterion: FocusCriterion,
  firstState: Exclude<FilterState, null> = "include",
): FocusQuery {
  const key = filterKey(criterion);
  const state = filterState(query, criterion);
  if (state === null) {
    return replaceFilter(query, key, {
      ...criterion,
      exclude: firstState === "exclude",
    });
  }
  if (state === firstState) {
    return replaceFilter(query, key, {
      ...criterion,
      exclude: firstState !== "exclude",
    });
  }
  return replaceFilter(query, key, null);
}

export function setFilter(
  query: FocusQuery,
  criterion: FocusCriterion,
  exclude = false,
): FocusQuery {
  return replaceFilter(query, filterKey(criterion), { ...criterion, exclude });
}

export function removeFilter(query: FocusQuery, key: string): FocusQuery {
  return replaceFilter(query, key, null);
}

export function toggleExclude(query: FocusQuery, key: string): FocusQuery {
  return {
    ...query,
    filters: query.filters.map((filter) =>
      filterKey(filter) === key
        ? { ...filter, exclude: !filter.exclude }
        : filter,
    ),
  };
}

export function isFocused(query: FocusQuery): boolean {
  return query.filters.length > 0 || Boolean(query.text?.trim());
}

export function formatLabel(value: string): string {
  const labels: Record<string, string> = {
    flac: "FLAC",
    alac: "ALAC",
    aac: "AAC",
    mp3: "MP3",
    vorbis: "Ogg Vorbis",
    opus: "Opus",
    wav: "WAV",
    aiff: "AIFF",
    aif: "AIFF",
    pcm: "PCM",
    wavpack: "WavPack",
    ape: "Monkey's Audio",
  };
  return labels[value.toLowerCase()] ?? value.toUpperCase();
}

export function qualityLabel(value: QualityTier | string): string {
  switch (value) {
    case "hires":
      return "Hi-Res";
    case "lossless":
      return "Lossless";
    case "cd":
      return "CD quality";
    case "lossy":
      return "Lossy";
    default:
      return "Unknown";
  }
}

export function sampleRateLabel(hz: number): string {
  const khz = hz / 1000;
  return Number.isInteger(khz) ? `${khz} kHz` : `${khz.toFixed(1)} kHz`;
}

function rangeLabel(
  min: number | null | undefined,
  max: number | null | undefined,
  format: (value: number) => string,
): string {
  if (min != null && max != null) {
    return min === max ? format(min) : `${format(min)}–${format(max)}`;
  }
  if (min != null) return `≥ ${format(min)}`;
  if (max != null) return `≤ ${format(max)}`;
  return "Any";
}

export function titleCase(value: string): string {
  return value ? value[0]!.toUpperCase() + value.slice(1) : value;
}

export const CREDIT_ROLE_LABELS: Record<string, [string, string]> = {
  any: ["Credit", "Credits"],
  conductor_ensemble: ["Conductor / Ensemble", "Conductor / Ensemble"],
  production: ["Production", "Production"],
  composer: ["Composer", "Composers"],
  lyricist: ["Lyricist", "Lyricists"],
  arranger: ["Arranger", "Arrangers"],
  conductor: ["Conductor", "Conductors"],
  performer: ["Performer", "Performers"],
  ensemble: ["Ensemble", "Ensembles"],
  producer: ["Producer", "Production"],
  remixer: ["Remixer", "Remixers"],
  mixer: ["Mixer", "Mixing"],
  engineer: ["Engineer", "Engineering"],
};

function yearRangeLabel(
  from: number | null | undefined,
  to: number | null | undefined,
): string {
  if (from != null && to != null && from % 10 === 0 && to === from + 9) {
    return `${from}s`;
  }
  return rangeLabel(from, to, String);
}

/** The text on a filter chip. */
export function criterionLabel(filter: FocusFilter): string {
  switch (filter.facet) {
    case "genre":
    case "label":
    case "form":
    case "instrumentation":
    case "period":
    case "nationality":
      return filter.value;
    case "composed":
      return `Composed ${yearRangeLabel(filter.from, filter.to)}`;
    case "work":
      return filter.name || "Composition";
    case "release_type":
      return titleCase(filter.value);
    case "format":
      return formatLabel(filter.value);
    case "quality":
      return qualityLabel(filter.value);
    case "year":
      return yearRangeLabel(filter.from, filter.to);
    case "sample_rate":
      return rangeLabel(filter.min, filter.max, sampleRateLabel);
    case "bit_depth":
      return rangeLabel(filter.min, filter.max, (bits) => `${bits}-bit`);
    case "credit": {
      const role = CREDIT_ROLE_LABELS[filter.role]?.[0] ?? filter.role;
      return `${filter.name || "Unknown"} · ${role}`;
    }
    case "artist":
      return filter.name || "Artist";
    case "album":
      return filter.name || "Album";
    case "tag":
      return `#${filter.name || filter.tag_id}`;
    case "source":
      return filter.name || "Storage";
    case "liked":
      return filter.exclude ? "Not liked" : "Liked";
    case "played":
      return filter.exclude ? "Unplayed" : "Played";
    case "added_within":
      return filter.days === 1 ? "Added today" : `Added in ${filter.days} days`;
  }
}

/** Whether a chip states its exclusion in its own label. */
export function labelCarriesExclusion(filter: FocusFilter): boolean {
  return filter.facet === "liked" || filter.facet === "played";
}

/** A one-line description, e.g. for a smart playlist created from a focus. */
export function focusSummary(query: FocusQuery): string {
  const label = (filter: FocusFilter) =>
    filter.exclude && !labelCarriesExclusion(filter)
      ? `not ${criterionLabel(filter)}`
      : criterionLabel(filter);
  const includes = query.filters
    .filter((filter) => !filter.exclude || labelCarriesExclusion(filter))
    .map(label);
  const excludes = query.filters
    .filter((filter) => filter.exclude && !labelCarriesExclusion(filter))
    .map(label);
  const parts = [
    includes.join(query.match === "any" ? " or " : " · "),
    ...excludes,
  ].filter(Boolean);
  if (query.text?.trim()) parts.push(`“${query.text.trim()}”`);
  return parts.join(" · ");
}

export function facetTitle(facet: Pick<Facet, "key" | "role">): string {
  switch (facet.key) {
    case "form":
      return "Form";
    case "instrumentation":
      return "Instrumentation";
    case "period":
      return "Period";
    case "composed":
      return "Composition date";
    case "nationality":
      return "Nationality";
    case "genre":
      return "Genres";
    case "year":
      return "Release date";
    case "format":
      return "Format";
    case "quality":
      return "Quality";
    case "sample_rate":
      return "Sample rate";
    case "bit_depth":
      return "Bit depth";
    case "artist":
      return "Artists";
    case "credit":
      return CREDIT_ROLE_LABELS[facet.role ?? "any"]?.[1] ?? "Credits";
    case "label":
      return "Labels";
    case "release_type":
      return "Release type";
    case "tag":
      return "Tags";
    case "source":
      return "Storage";
    case "status":
      return "Listening";
  }
}

/** Builds the criterion a click on a facet value applies. */
export function criterionForFacetValue(
  facet: Pick<Facet, "key" | "role">,
  value: FacetValue,
): FocusCriterion {
  switch (facet.key) {
    case "form":
    case "instrumentation":
    case "period":
    case "nationality":
      return { facet: facet.key, value: value.value };
    case "composed": {
      const year = Number(value.value);
      return { facet: "composed", from: year, to: year };
    }
    case "genre":
      return { facet: "genre", value: value.label };
    case "year": {
      const year = Number(value.value);
      return { facet: "year", from: year, to: year };
    }
    case "format":
      return { facet: "format", value: value.value };
    case "quality":
      return { facet: "quality", value: value.value as QualityTier };
    case "sample_rate": {
      const hz = Number(value.value);
      return { facet: "sample_rate", min: hz, max: hz };
    }
    case "bit_depth": {
      const bits = Number(value.value);
      return { facet: "bit_depth", min: bits, max: bits };
    }
    case "artist":
      return { facet: "artist", artist_id: value.value, name: value.label };
    case "credit":
      return {
        facet: "credit",
        role: (facet.role ?? "any") as Extract<
          FocusCriterion,
          { facet: "credit" }
        >["role"],
        artist_id: value.value,
        name: value.label,
      };
    case "label":
      return { facet: "label", value: value.value };
    case "release_type":
      return { facet: "release_type", value: value.value };
    case "tag":
      return { facet: "tag", tag_id: Number(value.value), name: value.label };
    case "source":
      return {
        facet: "source",
        root_id: Number(value.value),
        name: value.label,
      };
    case "status":
      return value.value === "liked" ? { facet: "liked" } : { facet: "played" };
  }
}

/** The filter state a facet value is shown with in the panel. */
export function facetValueState(
  query: FocusQuery,
  facet: Pick<Facet, "key" | "role">,
  value: FacetValue,
): FilterState {
  const criterion = criterionForFacetValue(facet, value);
  const state = filterState(query, criterion);
  if (facet.key === "status" && value.value !== "liked") {
    // "Played" and "Unplayed" are the two sides of one filter.
    if (value.value === "unplayed") {
      return state === "exclude" ? "include" : null;
    }
    return state === "include" ? "include" : null;
  }
  return state;
}

/** Applies a click on a facet value, including the Played/Unplayed pair. */
export function toggleFacetValue(
  query: FocusQuery,
  facet: Pick<Facet, "key" | "role">,
  value: FacetValue,
): FocusQuery {
  const criterion = criterionForFacetValue(facet, value);
  if (facet.key === "status" && value.value !== "liked") {
    const wantExclude = value.value === "unplayed";
    const state = filterState(query, criterion);
    if (state === (wantExclude ? "exclude" : "include")) {
      return removeFilter(query, filterKey(criterion));
    }
    return setFilter(query, criterion, wantExclude);
  }
  return cycleFilter(query, criterion);
}

export interface DecadeBucket {
  decade: number;
  count: number;
}

/** Groups per-year counts into decades, oldest first. */
export function decadesFromYears(values: FacetValue[]): DecadeBucket[] {
  const decades = new Map<number, number>();
  for (const value of values) {
    const year = Number(value.value);
    if (!Number.isFinite(year) || year <= 0) continue;
    const decade = Math.floor(year / 10) * 10;
    decades.set(decade, (decades.get(decade) ?? 0) + value.count);
  }
  return Array.from(decades, ([decade, count]) => ({ decade, count })).sort(
    (left, right) => left.decade - right.decade,
  );
}

export function decadeCriterion(
  decade: number,
  facet: "year" | "composed" = "year",
): FocusCriterion {
  return { facet, from: decade, to: decade + 9 };
}

/** Whether a facet is a year histogram (release or composition date). */
export function isYearFacet(key: FacetKey): key is "year" | "composed" {
  return key === "year" || key === "composed";
}

export interface SortOption {
  value: FocusSort;
  label: string;
}

export const SORT_OPTIONS: Record<FocusEntity, SortOption[]> = {
  albums: [
    { value: "artist", label: "Artist" },
    { value: "title", label: "Album title" },
    { value: "release_date", label: "Release date" },
    { value: "date_added", label: "Date added" },
    { value: "most_played", label: "Most played" },
    { value: "last_played", label: "Recently played" },
    { value: "quality", label: "Audio quality" },
    { value: "track_count", label: "Track count" },
    { value: "duration", label: "Length" },
    { value: "random", label: "Random" },
  ],
  songs: [
    { value: "artist", label: "Artist" },
    { value: "title", label: "Title" },
    { value: "album", label: "Album" },
    { value: "release_date", label: "Release date" },
    { value: "date_added", label: "Date added" },
    { value: "most_played", label: "Most played" },
    { value: "last_played", label: "Recently played" },
    { value: "quality", label: "Audio quality" },
    { value: "duration", label: "Length" },
    { value: "random", label: "Random" },
  ],
  artists: [
    { value: "title", label: "Name" },
    { value: "track_count", label: "Track count" },
    { value: "date_added", label: "Date added" },
    { value: "most_played", label: "Most played" },
    { value: "last_played", label: "Recently played" },
    { value: "random", label: "Random" },
  ],
  composers: [
    { value: "title", label: "Name" },
    { value: "release_date", label: "Chronological" },
    { value: "track_count", label: "Tracks composed" },
    { value: "date_added", label: "Date added" },
    { value: "most_played", label: "Most played" },
    { value: "last_played", label: "Recently played" },
    { value: "random", label: "Random" },
  ],
  compositions: [
    { value: "title", label: "Title" },
    { value: "artist", label: "Composer" },
    { value: "release_date", label: "Composition date" },
    { value: "track_count", label: "Recordings" },
    { value: "date_added", label: "Date added" },
    { value: "most_played", label: "Most played" },
    { value: "last_played", label: "Recently played" },
    { value: "random", label: "Random" },
  ],
};

const DESCENDING_BY_DEFAULT = new Set<FocusSort>([
  "date_added",
  "most_played",
  "last_played",
  "track_count",
  "quality",
]);

export function effectiveSort(query: FocusQuery): FocusSort {
  const options = SORT_OPTIONS[query.entity];
  if (query.sort && options.some((option) => option.value === query.sort)) {
    return query.sort;
  }
  return options[0]!.value;
}

export function effectiveDirection(query: FocusQuery): "asc" | "desc" {
  return (
    query.direction ??
    (DESCENDING_BY_DEFAULT.has(effectiveSort(query)) ? "desc" : "asc")
  );
}

/** Switches browsers while keeping filters; drops a sort the browser lacks. */
export function withEntity(query: FocusQuery, entity: FocusEntity): FocusQuery {
  const sortFits = SORT_OPTIONS[entity].some(
    (option) => option.value === query.sort,
  );
  return {
    ...query,
    entity,
    sort: sortFits ? query.sort : null,
    direction: sortFits ? query.direction : null,
  };
}

const FACETS: FacetKey[] = [
  "form",
  "instrumentation",
  "period",
  "composed",
  "nationality",
  "genre",
  "year",
  "format",
  "quality",
  "sample_rate",
  "bit_depth",
  "artist",
  "credit",
  "label",
  "release_type",
  "tag",
  "source",
  "status",
];

function isCriterion(value: unknown): value is FocusFilter {
  if (!value || typeof value !== "object") return false;
  const facet = (value as { facet?: unknown }).facet;
  return (
    typeof facet === "string" &&
    (FACETS.includes(facet as FacetKey) ||
      facet === "liked" ||
      facet === "played" ||
      facet === "album" ||
      facet === "work" ||
      facet === "added_within")
  );
}

/** Serializes a focus for a URL query parameter. */
export function encodeFocus(query: FocusQuery): string {
  const compact: Partial<FocusQuery> = { filters: query.filters };
  if (query.match === "any") compact.match = "any";
  if (query.text?.trim()) compact.text = query.text.trim();
  if (query.sort) compact.sort = query.sort;
  if (query.direction) compact.direction = query.direction;
  if (query.seed != null) compact.seed = query.seed;
  return JSON.stringify(compact);
}

/** Reads a focus from a URL parameter; malformed input yields no filters. */
export function decodeFocus(
  value: string | null | undefined,
  entity: FocusEntity,
): FocusQuery {
  const empty = emptyFocus(entity);
  if (!value) return empty;
  try {
    const parsed = JSON.parse(value) as Partial<FocusQuery>;
    if (!parsed || typeof parsed !== "object") return empty;
    return {
      entity,
      filters: Array.isArray(parsed.filters)
        ? parsed.filters.filter(isCriterion).slice(0, 32)
        : [],
      match: parsed.match === "any" ? "any" : undefined,
      text: typeof parsed.text === "string" ? parsed.text.slice(0, 200) : null,
      sort: typeof parsed.sort === "string" ? parsed.sort : null,
      direction:
        parsed.direction === "asc" || parsed.direction === "desc"
          ? parsed.direction
          : null,
      seed: typeof parsed.seed === "number" ? parsed.seed : null,
    };
  } catch {
    return empty;
  }
}

/** Facet columns in panel order, with credit roles that have values. */
export function visibleFacets(
  facets: Facet[],
  entity: FocusEntity,
  options: { composerScoped?: boolean; query?: FocusQuery } = {},
): Facet[] {
  const { query } = options;
  return facets
    .map((facet) => {
      // Values with an active filter stay so an exclusion can be cleared
      // where it was set, and the column does not vanish under the pointer.
      const values = facet.values.filter(
        (value) =>
          value.count > 0 ||
          (query !== undefined &&
            facetValueState(query, facet, value) !== null),
      );
      return {
        ...facet,
        values,
        // Zero-count values are hidden, so they do not count as "more".
        total: Math.max(
          values.length,
          facet.total - (facet.values.length - values.length),
        ),
      };
    })
    .filter((facet) => {
      if (facet.values.length === 0) return false;
      // A single storage location or bit depth is not a useful choice.
      if (
        (facet.key === "source" ||
          facet.key === "bit_depth" ||
          facet.key === "sample_rate" ||
          facet.key === "release_type") &&
        facet.values.length < 2
      ) {
        return false;
      }
      if (entity === "artists" && facet.key === "artist") return false;
      // The composer column only repeats the composers being browsed.
      if (
        (entity === "composers" || options.composerScoped) &&
        facet.key === "credit"
      ) {
        return facet.role !== "composer";
      }
      // One composer's works share their nationality.
      if (options.composerScoped && facet.key === "nationality") return false;
      // Works are browsed by their own facts, not album genres and dates.
      if (
        entity === "compositions" &&
        (facet.key === "genre" ||
          facet.key === "year" ||
          facet.key === "artist" ||
          facet.key === "label" ||
          facet.key === "release_type")
      ) {
        return false;
      }
      return true;
    });
}

/**
 * The query facet counts are computed over. With "match any", included
 * filters are alternatives, so the panel counts every value the exclusions
 * and text leave available instead of only the current matches.
 */
export function facetBaseQuery(query: FocusQuery): FocusQuery {
  if (query.match !== "any") return query;
  return {
    ...query,
    filters: query.filters.filter((filter) => filter.exclude),
  };
}

const NOUNS: Record<FocusEntity, [string, string]> = {
  albums: ["album", "albums"],
  songs: ["song", "songs"],
  artists: ["artist", "artists"],
  composers: ["composer", "composers"],
  compositions: ["composition", "compositions"],
};

export function entityNoun(entity: FocusEntity, count: number): string {
  const [one, many] = NOUNS[entity];
  return count === 1 ? one : many;
}

/** Header count, Roon style: "1 of 17 albums" while focused. */
export function focusCountLabel(
  query: FocusQuery,
  total: number | undefined,
  libraryTotal: number | undefined,
): string | null {
  if (total === undefined) return null;
  const format = (value: number) => value.toLocaleString("en-US");
  if (
    isFocused(query) &&
    libraryTotal !== undefined &&
    libraryTotal !== total
  ) {
    return `${format(total)} of ${format(libraryTotal)} ${entityNoun(query.entity, libraryTotal)}`;
  }
  return `${format(total)} ${entityNoun(query.entity, total)}`;
}

/**
 * The plain catalog endpoint keeps the existing library order and artwork
 * rules; any filter, text or non-default sort switches to the focus API.
 */
export function usesFocusEndpoint(query: FocusQuery): boolean {
  return (
    isFocused(query) ||
    query.entity === "composers" ||
    query.entity === "compositions" ||
    Boolean(query.scope?.length) ||
    Boolean(query.sort && query.sort !== defaultSortFor(query.entity)) ||
    Boolean(query.direction)
  );
}

function defaultSortFor(entity: FocusEntity) {
  return SORT_OPTIONS[entity][0]!.value;
}

/**
 * Keeps an album's own track order while showing only the tracks a focus
 * matched; an explicit sort uses the focus order instead.
 */
export function focusedAlbumTracks<T extends { id: string }>(
  tracks: T[],
  focusedIds: string[] | undefined,
  sorted: boolean,
): T[] {
  if (!focusedIds) return tracks;
  if (sorted) {
    const byId = new Map(tracks.map((track) => [track.id, track]));
    return focusedIds
      .map((id) => byId.get(id))
      .filter((track): track is T => Boolean(track));
  }
  const keep = new Set(focusedIds);
  return tracks.filter((track) => keep.has(track.id));
}

/**
 * One line about a work, Roon style: "Symphony for Orchestra · Composed
 * 1812". The specific scoring ("Violin & Orchestra") wins over the
 * instrumentation category when known.
 */
export function workDescription(work: WorkFacts): string {
  const scoring = work.scoring || work.instrumentation;
  const shape = work.form
    ? scoring
      ? `${work.form} for ${scoring}`
      : work.form
    : (scoring ?? null);
  return [shape, work.composedYear ? `Composed ${work.composedYear}` : null]
    .filter(Boolean)
    .join(" · ");
}

/** A composer's nationality, period and life dates, e.g. "German · Classical · 1770–1827". */
export function composerFacts(composer: {
  nationality?: string | null;
  period?: string | null;
  birthYear?: number | null;
  deathYear?: number | null;
}): string {
  const life = composer.birthYear
    ? composer.deathYear
      ? `${composer.birthYear}–${composer.deathYear}`
      : `b. ${composer.birthYear}`
    : null;
  return [composer.nationality, composer.period, life]
    .filter(Boolean)
    .join(" · ");
}
