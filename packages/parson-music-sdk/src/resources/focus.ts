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

export interface UserTag {
  id: number;
  name: string;
  item_count: number;
  created_at: string;
}

export type TaggedEntity = "track" | "album" | "artist";

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

export async function createTag(name: string): Promise<UserTag> {
  const response = await api.post<UserTag>("/library/tags", {
    name: name.trim(),
  });
  return response.data;
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
