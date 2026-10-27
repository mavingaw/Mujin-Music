import api from "../core/http";
import type { AudioProperties } from "../domain/types";
import type {
  LibraryCatalogAlbum,
  LibraryCatalogArtist,
  LibraryCatalogSong,
} from "./library";

/** The library browser a focus applies to. */
export type FocusEntity =
  "albums" | "songs" | "artists" | "composers" | "compositions";

/** Audio quality tiers derived from codec, sample rate and bit depth. */
export type QualityTier = "hires" | "lossless" | "cd" | "lossy";

/**
 * Canonical credit roles the indexer stores, `any`, and the merged facet
 * columns `conductor_ensemble` (conductor or ensemble) and `production`
 * (producer, engineer, mixer or remixer).
 */
export type FocusCreditRole =
  | "any"
  | "conductor_ensemble"
  | "production"
  | "composer"
  | "lyricist"
  | "arranger"
  | "conductor"
  | "performer"
  | "ensemble"
  | "producer"
  | "remixer"
  | "mixer"
  | "engineer";

/**
 * One focus criterion. `name` fields are display hints so saved focuses can
 * render their chips without another lookup; they never affect matching.
 */
export type FocusCriterion =
  | { facet: "genre"; value: string }
  | { facet: "year"; from?: number | null; to?: number | null }
  | { facet: "format"; value: string }
  | { facet: "quality"; value: QualityTier }
  | { facet: "sample_rate"; min?: number | null; max?: number | null }
  | { facet: "bit_depth"; min?: number | null; max?: number | null }
  | {
      facet: "credit";
      role: FocusCreditRole;
      artist_id: string;
      name?: string | null;
    }
  | { facet: "artist"; artist_id: string; name?: string | null }
  | { facet: "album"; album_id: string; name?: string | null }
  | { facet: "label"; value: string }
  | { facet: "release_type"; value: string }
  | { facet: "tag"; tag_id: number; name?: string | null }
  | { facet: "source"; root_id: number; name?: string | null }
  | { facet: "liked" }
  | { facet: "played" }
  | { facet: "added_within"; days: number }
  /** Work form: Symphony, Sonata, Concerto, String Quartet, Song, ... */
  | { facet: "form"; value: string }
  /** Instrumentation: Orchestra, Solo Keyboard, Keyboard Trio, ... */
  | { facet: "instrumentation"; value: string }
  /** Musical period; a composer's own period in the Composers browser. */
  | { facet: "period"; value: string }
  /** Year of composition. */
  | { facet: "composed"; from?: number | null; to?: number | null }
  /** Nationality of a composer. */
  | { facet: "nationality"; value: string }
  /** The recordings of one work. */
  | { facet: "work"; work_id: string; name?: string | null };

/** A criterion that either keeps (include) or removes (exclude) matches. */
export type FocusFilter = FocusCriterion & { exclude?: boolean };

export type FocusSort =
  | "artist"
  | "title"
  | "album"
  | "release_date"
  | "date_added"
  | "most_played"
  | "last_played"
  | "duration"
  | "track_count"
  | "quality"
  | "random";

export type SortDirection = "asc" | "desc";

export interface FocusQuery {
  entity: FocusEntity;
  /** How included filters combine; exclusions always apply. */
  match?: "all" | "any";
  filters: FocusFilter[];
  /** Case-insensitive text filter over titles and artist names. */
  text?: string | null;
  sort?: FocusSort | null;
  direction?: SortDirection | null;
  /** Seed for a stable random order while paging. */
  seed?: number | null;
  /**
   * Criteria that fix the subset being browsed (a composer's works, an
   * album's tracks). They always apply, are not chips, and bound the
   * "of N" total.
   */
  scope?: FocusCriterion[];
}

export type FocusQualityBadge = "hires" | "cd" | "lossy";

export interface FocusAlbum extends LibraryCatalogAlbum {
  quality: FocusQualityBadge | null;
}

export interface FocusSong extends LibraryCatalogSong {
  audio: AudioProperties | null;
  quality: FocusQualityBadge | null;
}

/** A person in the Composers browser. */
export interface FocusComposer {
  id: string;
  name: string;
  trackCount: number;
  /** Distinct compositions they wrote. */
  workCount: number;
  albumCount: number;
  birthYear?: number | null;
  deathYear?: number | null;
  nationality?: string | null;
  period?: string | null;
}

/** Facts about a work, shared by browser rows and grouped compositions. */
export interface WorkFacts {
  form?: string | null;
  /** Facet category: Orchestra, Solo Keyboard, Keyboard Trio, ... */
  instrumentation?: string | null;
  /** Specific scoring when known, e.g. "Violin & Orchestra". */
  scoring?: string | null;
  composedYear?: number | null;
  period?: string | null;
}

/** A row in the Compositions browser. */
export interface FocusWork extends WorkFacts {
  id: string;
  title: string;
  composers: NamedRef[];
  /** Albums holding a recording of the work. */
  recordingCount: number;
  trackCount: number;
  /** Whether the title comes from a WORK tag or MusicBrainz. */
  tagged: boolean;
  musicbrainzWorkId?: string | null;
}

export interface FocusPage {
  entity: FocusEntity;
  /** Entities that match the focus. */
  total: number;
  /** Entities in the library before the focus applies. */
  libraryTotal: number;
  offset: number;
  albums: FocusAlbum[];
  songs: FocusSong[];
  artists: LibraryCatalogArtist[];
  composers?: FocusComposer[];
  compositions?: FocusWork[];
}

export interface NamedRef {
  id: string;
  name: string;
}

export interface CompositionTrack {
  id: string;
  title: string;
  /** From MOVEMENTNAME, or the title after "Work: ". */
  movement: string | null;
  movementNumber: number | null;
  durationSeconds: number;
}

/** One album's performance of a composition. */
export interface CompositionRecording {
  albumId: string | null;
  albumName: string | null;
  artistId: string | null;
  artistName: string | null;
  /** Album artwork path for `/media/images/`. */
  coverPath: string | null;
  releaseYear: string | null;
  /** Conductors, ensembles and soloists. */
  performers: string[];
  durationSeconds: number;
  tracks: CompositionTrack[];
}

export interface Composition extends WorkFacts {
  /** The work id. */
  key: string;
  title: string;
  /** Whether the title comes from a WORK tag or MusicBrainz. */
  tagged: boolean;
  composers: NamedRef[];
  movementCount: number;
  recordings: CompositionRecording[];
}

export interface FocusCompositions {
  total: number;
  trackTotal: number;
  truncated: boolean;
  compositions: Composition[];
}

export interface FocusPersonRole {
  /** `primary`, or a credit facet column such as `composer`. */
  role: string;
  track_count: number;
}

export interface FocusPerson {
  id: string;
  name: string;
  roles: FocusPersonRole[];
  composer: FocusComposer | null;
}

export type FacetKey =
  | "form"
  | "instrumentation"
  | "period"
  | "composed"
  | "nationality"
  | "genre"
  | "year"
  | "format"
  | "quality"
  | "sample_rate"
  | "bit_depth"
  | "artist"
  | "credit"
  | "label"
  | "release_type"
  | "tag"
  | "source"
  | "status";

export interface FacetValue {
  value: string;
  label: string;
  count: number;
}

export interface Facet {
  key: FacetKey;
  /** Credit role for `credit` facets (one facet per role). */
  role?: string;
  /** Distinct values before the per-facet limit. */
  total: number;
  values: FacetValue[];
}

export interface FocusFacets {
  total: number;
  facets: Facet[];
}

export interface FacetOptions {
  only?: FacetKey[];
  search?: string;
  limit?: number;
}

export interface SavedFocus {
  id: number;
  name: string;
  view: FocusEntity;
  query: FocusQuery;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface SmartPlaylistInfo {
  query: FocusQuery;
  track_limit: number;
  refreshed_at: string;
}

export interface SmartPlaylist {
  id: number;
  name: string;
  song_count: number;
  smart: SmartPlaylistInfo;
}

/** Colours a tag can carry; clients map each to their own palette. */
export const TAG_COLORS = [
  "slate",
  "red",
  "orange",
  "amber",
  "lime",
  "green",
  "teal",
  "sky",
  "blue",
  "violet",
  "fuchsia",
  "pink",
] as const;

export type TagColor = (typeof TAG_COLORS)[number];

export type TaggedEntity = "track" | "album" | "artist" | "work" | "playlist";

/** Items of each kind on a tag. A smart tag counts what its focus matches. */
export type TagCounts = Record<TaggedEntity, number>;

export interface UserTag {
  id: number;
  name: string;
  item_count: number;
  created_at: string;
  updated_at?: string;
  /** A smart tag holds whatever its focus matches and updates by itself. */
  kind?: "manual" | "smart";
  color?: TagColor | null;
  description?: string | null;
  /** The focus a smart tag follows. */
  query?: FocusQuery | null;
  counts?: TagCounts;
}

export interface TagDetails {
  color?: TagColor | null;
  description?: string | null;
  /** Makes the tag smart. Albums, songs and artists browsers only. */
  query?: FocusQuery | null;
}

export interface TagPatch {
  name?: string;
  color?: TagColor | null;
  description?: string | null;
  /** A new focus for a smart tag. */
  query?: FocusQuery;
}

export interface TaggedPlaylist {
  id: number;
  name: string;
  songCount: number;
}

/** A tag with its members, hand-tagged or matched by its focus. */
export interface TagPage {
  tag: UserTag;
  albums: FocusAlbum[];
  songs: FocusSong[];
  artists: LibraryCatalogArtist[];
  works: FocusWork[];
  playlists: TaggedPlaylist[];
}

export interface TagCoverage {
  tagId: number;
  /** How many of the requested items carry the tag. */
  count: number;
}

export interface BulkTagOutcome {
  added: number;
  removed: number;
  /** Tags skipped because they are missing or smart. */
  skipped: number[];
}

const MAX_TAG_BATCH = 500;

export async function getFocusPage(
  query: FocusQuery,
  offset = 0,
  limit = 100,
): Promise<FocusPage> {
  const response = await api.post<FocusPage>("/library/focus", {
    query,
    offset,
    limit,
  });
  return response.data;
}

/** Groups a focus's tracks into compositions with their recordings. */
export async function getFocusCompositions(
  query: FocusQuery,
): Promise<FocusCompositions> {
  const response = await api.post<FocusCompositions>(
    "/library/focus/compositions",
    { query },
  );
  return response.data;
}

/** A credited person (composer, conductor, performer) and their roles. */
export async function getFocusPerson(id: string): Promise<FocusPerson> {
  const response = await api.get<FocusPerson>(
    `/library/focus/people/${encodeURIComponent(id)}`,
  );
  return response.data;
}

export async function getFocusFacets(
  query: FocusQuery,
  options: FacetOptions = {},
): Promise<FocusFacets> {
  const response = await api.post<FocusFacets>("/library/focus/facets", {
    query,
    only: options.only ?? [],
    search: options.search?.trim() || undefined,
    limit: options.limit,
  });
  return response.data;
}

export async function getSavedFocuses(): Promise<SavedFocus[]> {
  const response = await api.get<SavedFocus[]>("/library/focus/saved");
  return Array.isArray(response.data) ? response.data : [];
}

export async function saveFocus(
  name: string,
  query: FocusQuery,
): Promise<SavedFocus> {
  const response = await api.post<SavedFocus>("/library/focus/saved", {
    name: name.trim(),
    query,
  });
  return response.data;
}

export async function updateSavedFocus(
  id: number,
  changes: { name?: string; query?: FocusQuery },
): Promise<SavedFocus> {
  const response = await api.patch<SavedFocus>(
    `/library/focus/saved/${id}`,
    changes,
  );
  return response.data;
}

export async function deleteSavedFocus(id: number): Promise<void> {
  await api.delete(`/library/focus/saved/${id}`);
}

export async function reorderSavedFocuses(
  ids: number[],
): Promise<SavedFocus[]> {
  const response = await api.put<SavedFocus[]>("/library/focus/saved/order", {
    ids,
  });
  return response.data;
}

export async function createSmartPlaylist(
  name: string,
  query: FocusQuery,
  options: { description?: string; limit?: number } = {},
): Promise<SmartPlaylist> {
  const response = await api.post<SmartPlaylist>("/library/focus/playlists", {
    name: name.trim(),
    query,
    description: options.description,
    limit: options.limit,
  });
  return response.data;
}

export async function refreshSmartPlaylist(id: number): Promise<SmartPlaylist> {
  const response = await api.post<SmartPlaylist>(
    `/library/focus/playlists/${id}/refresh`,
  );
  return response.data;
}

/** Keeps the playlist's current tracks and stops following its focus. */
export async function detachSmartPlaylist(id: number): Promise<void> {
  await api.delete(`/library/focus/playlists/${id}/smart`);
}

export async function getTags(): Promise<UserTag[]> {
  const response = await api.get<UserTag[]>("/library/tags");
  return Array.isArray(response.data) ? response.data : [];
}

export async function createTag(
  name: string,
  details: TagDetails = {},
): Promise<UserTag> {
  const response = await api.post<UserTag>("/library/tags", {
    name: name.trim(),
    ...details,
    description: details.description?.trim() || undefined,
  });
  return response.data;
}

/** Creates a smart tag that holds whatever `query` matches. */
export function createSmartTag(
  name: string,
  query: FocusQuery,
  details: Omit<TagDetails, "query"> = {},
): Promise<UserTag> {
  return createTag(name, { ...details, query });
}

export async function updateTag(id: number, patch: TagPatch): Promise<UserTag> {
  const response = await api.patch<UserTag>(`/library/tags/${id}`, {
    ...patch,
    name: patch.name?.trim(),
  });
  return response.data;
}

export async function getTagPage(id: number): Promise<TagPage> {
  const response = await api.get<TagPage>(`/library/tags/${id}`);
  return response.data;
}

/** For each tag, how many of the given items carry it. */
export async function getTagCoverage(
  entityType: TaggedEntity,
  entityIds: string[],
): Promise<Map<number, number>> {
  const unique = Array.from(new Set(entityIds.filter(Boolean)));
  const coverage = new Map<number, number>();
  for (let index = 0; index < unique.length; index += MAX_TAG_BATCH) {
    const response = await api.post<TagCoverage[]>(
      "/library/tags/assigned/bulk",
      {
        entity_type: entityType,
        entity_ids: unique.slice(index, index + MAX_TAG_BATCH),
      },
    );
    for (const item of Array.isArray(response.data) ? response.data : []) {
      coverage.set(item.tagId, (coverage.get(item.tagId) ?? 0) + item.count);
    }
  }
  return coverage;
}

/** Adds and removes several tags on many items, as Manage Tags saves. */
export async function bulkTag(
  entityType: TaggedEntity,
  entityIds: string[],
  changes: { add?: number[]; remove?: number[] },
): Promise<BulkTagOutcome> {
  const unique = Array.from(new Set(entityIds.filter(Boolean)));
  const outcome: BulkTagOutcome = { added: 0, removed: 0, skipped: [] };
  const add = changes.add ?? [];
  const remove = changes.remove ?? [];
  if (!unique.length || (!add.length && !remove.length)) return outcome;
  for (let index = 0; index < unique.length; index += MAX_TAG_BATCH) {
    const response = await api.post<BulkTagOutcome>("/library/tags/bulk", {
      entity_type: entityType,
      entity_ids: unique.slice(index, index + MAX_TAG_BATCH),
      add,
      remove,
    });
    outcome.added += response.data?.added ?? 0;
    outcome.removed += response.data?.removed ?? 0;
    for (const id of response.data?.skipped ?? []) {
      if (!outcome.skipped.includes(id)) outcome.skipped.push(id);
    }
  }
  return outcome;
}

export type TagCheckState = "checked" | "mixed" | "unchecked";

/** Tri-state of one tag across a selection of `total` items. */
export function tagCheckState(count: number, total: number): TagCheckState {
  if (total <= 0 || count <= 0) return "unchecked";
  return count >= total ? "checked" : "mixed";
}

/**
 * What Manage Tags must change: tags the listener ticked that not every
 * item has yet, and tags they cleared that some item still has. Mixed tags
 * left alone stay as they are.
 */
export function tagChanges(
  initial: Map<number, TagCheckState>,
  desired: Map<number, TagCheckState>,
): { add: number[]; remove: number[] } {
  const add: number[] = [];
  const remove: number[] = [];
  for (const [id, state] of desired) {
    const before = initial.get(id) ?? "unchecked";
    if (state === before) continue;
    if (state === "checked") add.push(id);
    else if (state === "unchecked" && before !== "unchecked") remove.push(id);
  }
  return { add, remove };
}

/** Short summary of what a tag holds: "12 albums · 3 artists". */
export function tagCountsSummary(tag: Pick<UserTag, "counts" | "item_count">) {
  const counts = tag.counts;
  if (!counts) {
    return `${tag.item_count} ${tag.item_count === 1 ? "item" : "items"}`;
  }
  const nouns: [TaggedEntity, string, string][] = [
    ["album", "album", "albums"],
    ["artist", "artist", "artists"],
    ["track", "track", "tracks"],
    ["work", "work", "works"],
    ["playlist", "playlist", "playlists"],
  ];
  const parts = nouns
    .filter(([key]) => counts[key] > 0)
    .map(
      ([key, one, many]) => `${counts[key]} ${counts[key] === 1 ? one : many}`,
    );
  return parts.length ? parts.join(" · ") : "Empty";
}

export async function renameTag(id: number, name: string): Promise<UserTag> {
  const response = await api.patch<UserTag>(`/library/tags/${id}`, {
    name: name.trim(),
  });
  return response.data;
}

export async function deleteTag(id: number): Promise<void> {
  await api.delete(`/library/tags/${id}`);
}

export async function getAssignedTags(
  entityType: TaggedEntity,
  entityId: string,
): Promise<number[]> {
  const response = await api.get<number[]>("/library/tags/assigned", {
    params: { entity_type: entityType, entity_id: entityId },
  });
  return Array.isArray(response.data) ? response.data : [];
}

async function changeTagItems(
  path: string,
  entityType: TaggedEntity,
  entityIds: string[],
): Promise<number> {
  const unique = Array.from(new Set(entityIds.filter(Boolean)));
  let changed = 0;
  for (let index = 0; index < unique.length; index += MAX_TAG_BATCH) {
    const response = await api.post<{ changed: number }>(path, {
      entity_type: entityType,
      entity_ids: unique.slice(index, index + MAX_TAG_BATCH),
    });
    changed += response.data?.changed ?? 0;
  }
  return changed;
}

export function tagItems(
  tagId: number,
  entityType: TaggedEntity,
  entityIds: string[],
): Promise<number> {
  return changeTagItems(`/library/tags/${tagId}/items`, entityType, entityIds);
}

export function untagItems(
  tagId: number,
  entityType: TaggedEntity,
  entityIds: string[],
): Promise<number> {
  return changeTagItems(
    `/library/tags/${tagId}/items/remove`,
    entityType,
    entityIds,
  );
}

export interface TagPreview {
  tagId: number;
  /** Up to four album covers for the tag's tile. */
  covers: string[];
}

/** Album covers that picture each tag, for the Tags overview. */
export async function getTagPreviews(): Promise<Map<number, string[]>> {
  const response = await api.get<TagPreview[]>("/library/tags/previews");
  return new Map(
    (Array.isArray(response.data) ? response.data : []).map((preview) => [
      preview.tagId,
      preview.covers ?? [],
    ]),
  );
}
