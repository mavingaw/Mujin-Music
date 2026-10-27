import api from "../core/http";

/**
 * Library tools (admins): skipped files, duplicates, album versions, the
 * Albumizer, rescans with progress, statistics and missing-file clean-up.
 * Every operation is non-destructive: music files are only ever read.
 */

const BASE = "/library/tools";

/**
 * `ai_enhanced`: a copy the AI enhancer made from a lossy file. Plausible
 * restored treble, never the original master, so never "lossless".
 */
export type FileQualityTier =
  "dsd" | "hi_res" | "lossless" | "ai_enhanced" | "lossy" | "unknown";

/** One quality scale for every copy: tier, comparable score and a label such as `FLAC 24/96`. */
export interface FileQuality {
  tier: FileQualityTier;
  score: number;
  label: string;
  /** Codec family, e.g. `FLAC`, `ALAC`, `DSD`, `MP3`. */
  format: string;
}

export interface ToolSong {
  id: string;
  title: string;
  artist: string;
  artist_id: string;
  album_id: string;
  album: string;
  cover_url: string;
  track_number: number;
  duration: number;
  path: string;
  quality: FileQuality;
}

export interface Breakdown {
  key: string;
  label: string;
  tracks: number;
}

export interface LibraryStats {
  tracks: number;
  albums: number;
  artists: number;
  duration_seconds: number;
  size_bytes: number;
  tiers: Breakdown[];
  formats: Breakdown[];
  sample_rates: Breakdown[];
  bit_depths: Breakdown[];
  identification: {
    identified: number;
    awaiting: number;
    unidentified: number;
    enabled: boolean;
  };
  analysis: { analyzed: number; partial: number; awaiting: number };
}

export type ToolJobKind =
  "rescan" | "reidentify" | "reanalyze" | "skipped_check" | "lossless_check";
export type ToolJobState = "running" | "completed" | "failed" | "cancelled";

export interface ToolJob {
  id: number;
  kind: ToolJobKind;
  title: string;
  scope: string;
  state: ToolJobState;
  /** 0 while the size of the job is not known yet. */
  total: number;
  done: number;
  failed: number;
  current: string;
  message: string;
  started_at_ms: number;
  finished_at_ms?: number;
}

export interface LibraryToolsOverview {
  stats: LibraryStats;
  skipped: number;
  skipped_checked_at: string | null;
  missing: number;
  duplicates: number;
  duplicates_suggested: number;
  multi_copy_tracks: number;
  version_groups: number;
  loose_tracks: number;
  user_albums: number;
  last_scan_at: string | null;
  jobs: ToolJob[];
}

export type SkipReason =
  | "corrupt"
  | "tags_unreadable"
  | "no_audio"
  | "empty_file"
  | "io_failure"
  | "unreadable_folder"
  | "not_yet_imported"
  | "unsupported_format"
  | "drm_protected"
  | "video_file"
  | "disc_image"
  | "playlist_file"
  | "unreadable_image";

export interface SkippedFix {
  summary: string;
  /** A button the app offers: `rescan`, `edit` or `import_playlist`. */
  action?: "rescan" | "edit" | "import_playlist";
  /** A command the user can copy and run themselves. */
  command?: string;
}

export interface SkippedFile {
  path: string;
  file_name: string;
  folder: string;
  root_path: string;
  reason: SkipReason;
  label: string;
  detail: string;
  extension: string;
  size_bytes: number;
  ignored: boolean;
  /** The file is in the library anyway (with reduced metadata). */
  imported: boolean;
  song_id?: string;
  album_id?: string;
  fix: SkippedFix;
  first_seen_at: string;
}

export interface SkippedFiles {
  items: SkippedFile[];
  reasons: { reason: SkipReason; label: string; count: number }[];
  checked_at: string | null;
  job: ToolJob | null;
}

export type DuplicateKind = "same_album" | "loose" | "cross_album";

export interface DuplicateGroup {
  key: string;
  kind: DuplicateKind;
  confidence: "high" | "medium";
  members: string[];
  best: string;
  /** Keeping only the best copy is safe to do in bulk. */
  suggested: boolean;
  evidence: {
    tags: boolean;
    isrc: boolean;
    fingerprint_similarity: number | null;
    duration_spread: number;
  };
  songs: ToolSong[];
}

export interface CopyFile {
  path: string;
  quality: FileQuality;
  size_bytes: number;
  duration: number;
  playing: boolean;
}

export interface MultiCopyTrack {
  song: ToolSong;
  files: CopyFile[];
  /** The user picked the copy that plays. */
  chosen: boolean;
}

export interface HiddenCopy {
  path: string;
  state: "hidden" | "merged";
  kept_path: string | null;
  song_id: string;
  title: string;
  album: string;
  updated_at: string;
  kept?: ToolSong;
}

export interface Duplicates {
  groups: DuplicateGroup[];
  copies: MultiCopyTrack[];
  hidden: HiddenCopy[];
  dismissed: number;
  fingerprints: {
    tracks: number;
    total: number;
    method: "chromaprint" | "envelope" | null;
  };
}

export type DuplicateAction = "merge" | "hide" | "dismiss";

export interface VersionAlbum {
  album_id: string;
  title: string;
  edition: string;
  preferred: boolean;
  hidden: boolean;
  year: string;
  track_count: number;
  duration: number;
  quality: FileQuality;
  mixed: boolean;
  cover_url: string;
  artist_id: string;
}

export interface VersionGroup {
  key: string;
  title: string;
  artist_name: string;
  preferred: string;
  chosen: boolean;
  hide_others: boolean;
  manual: boolean;
  versions: VersionAlbum[];
}

export interface LooseTrack {
  song_id: string;
  title: string;
  artist: string;
  album_id: string;
  album: string;
  track_number: number;
  duration: number;
  cover_url: string;
}

export interface AlbumSuggestion {
  folder: string;
  title: string;
  artist: string;
  tracks: LooseTrack[];
}

export type UserAlbumType =
  "album" | "ep" | "single" | "compilation" | "live" | "soundtrack";

export interface UserAlbum {
  id: string;
  title: string;
  artist_id: string | null;
  artist_name: string;
  year: string;
  primary_type: UserAlbumType;
  cover_url: string;
  tracks: ToolSong[];
}

export interface AlbumizerState {
  albums: UserAlbum[];
  suggestions: AlbumSuggestion[];
}

export interface AlbumizeRequest {
  /** Present to edit an album made earlier. */
  id?: string;
  title: string;
  artist_name: string;
  artist_id?: string;
  year?: string;
  primary_type?: UserAlbumType;
  /** Use this track's album cover. */
  cover_song_id?: string;
  /** Tracks in album order. */
  song_ids: string[];
}

export interface MissingFile {
  path: string;
  root_path: string;
  title: string;
  album: string;
  artist: string;
  size_bytes: number;
  last_seen_at: string;
  /** False while the drive or share holding it is offline; such files are always kept. */
  root_reachable: boolean;
  present_again: boolean;
}

export interface MissingFiles {
  files: MissingFile[];
  roots: { path: string; reachable: boolean }[];
  forgettable: number;
}

export type ToolScope =
  | { scope: "library" }
  | { scope: "artists"; ids: string[] }
  | { scope: "albums"; ids: string[] }
  | { scope: "songs"; ids: string[] };

export interface SongAnalysis {
  song: ToolSong;
  size_bytes: number;
  waveform: { peaks: number[]; rms: number[] } | null;
  fingerprint: "chromaprint" | "envelope" | null;
  analyzed_at: string | null;
  analysis_error: string | null;
  /** The file changed since it was analyzed. */
  stale: boolean;
  loudness: { integrated_lufs: number; true_peak_db: number } | null;
  replay_gain: {
    track_gain_db?: number;
    track_peak?: number;
    album_gain_db?: number;
    album_peak?: number;
  } | null;
  copies: CopyFile[];
}

export async function getLibraryToolsOverview(): Promise<LibraryToolsOverview> {
  const response = await api.get<LibraryToolsOverview>(`${BASE}/overview`);
  return response.data;
}

export async function getToolJobs(): Promise<ToolJob[]> {
  const response = await api.get<{ jobs: ToolJob[] }>(`${BASE}/jobs`);
  return Array.isArray(response.data?.jobs) ? response.data.jobs : [];
}

export async function cancelToolJob(id: number): Promise<void> {
  await api.post(`${BASE}/jobs/${encodeURIComponent(String(id))}/cancel`);
}

export async function getSkippedFiles(): Promise<SkippedFiles> {
  const response = await api.get<SkippedFiles>(`${BASE}/skipped`);
  return response.data;
}

export async function checkSkippedFiles(): Promise<ToolJob> {
  const response = await api.post<ToolJob>(`${BASE}/skipped/check`);
  return response.data;
}

export async function ignoreSkippedFiles(
  paths: string[],
  ignored = true,
): Promise<void> {
  await api.post(`${BASE}/skipped/ignore`, { paths, ignored });
}

export async function retrySkippedFiles(paths: string[]): Promise<ToolJob> {
  const response = await api.post<ToolJob>(`${BASE}/skipped/retry`, {
    paths,
  });
  return response.data;
}

/** A skipped playlist file's text, ready for `createImportJob`. */
export async function getSkippedPlaylist(
  path: string,
): Promise<{ file_name: string; content: string }> {
  const response = await api.get<{ file_name: string; content: string }>(
    `${BASE}/skipped/playlist`,
    { params: { path } },
  );
  return response.data;
}

export async function getDuplicates(): Promise<Duplicates> {
  const response = await api.get<Duplicates>(`${BASE}/duplicates`);
  return response.data;
}

/**
 * `merge` keeps one copy (the best unless `keepSongId` is given) and moves
 * likes and playlist entries to it; `hide` hides `songIds`; `dismiss` marks
 * the group as not duplicates.
 */
export async function resolveDuplicate(
  key: string,
  action: DuplicateAction,
  options: { keepSongId?: string; songIds?: string[] } = {},
): Promise<void> {
  await api.post(`${BASE}/duplicates/resolve`, {
    key,
    action,
    keep_song_id: options.keepSongId,
    song_ids: options.songIds,
  });
}

/** Keeps the highest quality copy in every group where that is safe. */
export async function resolveSuggestedDuplicates(
  action: "merge" | "hide" = "merge",
): Promise<{ groups: number; changed: number }> {
  const response = await api.post<{ groups: number; changed: number }>(
    `${BASE}/duplicates/resolve-suggested`,
    { action },
  );
  return response.data;
}

export async function restoreHiddenCopies(paths: string[]): Promise<void> {
  await api.post(`${BASE}/duplicates/restore`, { paths });
}

/** Picks the copy of a track that plays; `null` returns to the best copy. */
export async function preferTrackCopy(
  trackId: string,
  path: string | null,
): Promise<void> {
  await api.post(`${BASE}/copies/prefer`, { track_id: trackId, path });
}

export async function getAlbumVersionGroups(): Promise<VersionGroup[]> {
  const response = await api.get<{ groups: VersionGroup[] }>(
    `${BASE}/versions`,
  );
  return Array.isArray(response.data?.groups) ? response.data.groups : [];
}

export async function getAlbumVersions(
  albumId: string,
): Promise<VersionGroup | null> {
  const response = await api.get<{ group: VersionGroup | null }>(
    `${BASE}/albums/${encodeURIComponent(albumId)}/versions`,
  );
  return response.data?.group ?? null;
}

/** `preferredAlbumId: null` returns the group to automatic (best quality, all shown). */
export async function chooseAlbumVersion(
  key: string,
  preferredAlbumId: string | null,
  hideOthers = false,
): Promise<void> {
  await api.post(`${BASE}/versions/choice`, {
    key,
    preferred_album_id: preferredAlbumId,
    hide_others: hideOthers,
  });
}

export async function groupAlbumVersions(albumIds: string[]): Promise<string> {
  const response = await api.post<{ key: string }>(`${BASE}/versions/group`, {
    album_ids: albumIds,
  });
  return response.data.key;
}

export async function ungroupAlbumVersions(albumIds: string[]): Promise<void> {
  await api.post(`${BASE}/versions/ungroup`, { album_ids: albumIds });
}

export async function getAlbumizer(): Promise<AlbumizerState> {
  const response = await api.get<AlbumizerState>(`${BASE}/albumizer`);
  return response.data;
}

export async function albumizeTracks(
  request: AlbumizeRequest,
): Promise<string> {
  const response = await api.post<{ album_id: string }>(
    `${BASE}/albumizer`,
    request,
  );
  return response.data.album_id;
}

/** Undoes an Albumizer album: its tracks return to where the scan put them. */
export async function deleteUserAlbum(id: string): Promise<void> {
  await api.delete(`${BASE}/albumizer/${encodeURIComponent(id)}`);
}

export async function getMissingFiles(): Promise<MissingFiles> {
  const response = await api.get<MissingFiles>(`${BASE}/missing`);
  return response.data;
}

/**
 * Forgets missing files. `confirm` must equal the number of files that will
 * be forgotten, so a library that changed since the review is not touched.
 */
export async function forgetMissingFiles(
  confirm: number,
  paths?: string[],
): Promise<number> {
  const response = await api.post<{ forgotten: number }>(
    `${BASE}/missing/forget`,
    { confirm, paths },
  );
  return response.data.forgotten;
}

export async function rescanLibraryItems(scope: ToolScope): Promise<ToolJob> {
  const response = await api.post<ToolJob>(`${BASE}/rescan`, scope);
  return response.data;
}

export async function reidentifyLibraryItems(
  scope: ToolScope,
): Promise<ToolJob> {
  const response = await api.post<ToolJob>(`${BASE}/reidentify`, scope);
  return response.data;
}

export async function reanalyzeLibraryItems(
  scope: ToolScope,
  force = false,
): Promise<ToolJob> {
  const response = await api.post<ToolJob>(`${BASE}/reanalyze`, {
    ...scope,
    force,
  });
  return response.data;
}

export async function getSongAnalysis(songId: string): Promise<SongAnalysis> {
  const response = await api.get<SongAnalysis>(
    `${BASE}/songs/${encodeURIComponent(songId)}/analysis`,
  );
  return response.data;
}

/** Fraction done for a progress bar, or `null` while indeterminate. */
export function toolJobProgress(job: ToolJob): number | null {
  if (job.state !== "running") return 1;
  if (job.total <= 0) return null;
  return Math.max(0, Math.min(1, job.done / job.total));
}
