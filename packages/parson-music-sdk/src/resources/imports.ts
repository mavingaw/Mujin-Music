import api, { isApiError } from "../core/http";

/**
 * Imports from Spotify, Apple Music, YouTube Music, Deezer and playlist
 * files. Parson never streams audio from these services: each service lists
 * what its terms allow as capabilities, and imported tracks are matched to
 * songs in the person's own library.
 */

export type ImportServiceId =
  "spotify" | "apple_music" | "youtube_music" | "deezer" | "file";

export type ProviderCapability =
  "metadata" | "library-import" | "playback" | "handoff";

export const PROVIDER_CAPABILITY_LABELS: Record<ProviderCapability, string> = {
  metadata: "Metadata",
  "library-import": "Library import",
  playback: "Playback",
  handoff: "Handoff",
};

export type ConnectMethod =
  "oauth_pkce" | "developer_token" | "api_key" | "public" | "upload";

export interface CapabilityGrant {
  capability: ProviderCapability;
  available: boolean;
  detail: string;
}

export interface ImportConnection {
  connected_at: string;
  account_label: string | null;
  /** Never contains a secret; hints such as "…cdef" only. */
  details: Record<string, unknown>;
}

export interface ImportService {
  id: ImportServiceId;
  name: string;
  connect_method: ConnectMethod;
  capabilities: CapabilityGrant[];
  accepts: string[];
  summary: string;
  requirements: string;
  terms_url: string;
  connection: ImportConnection | null;
}

export interface RemotePlaylist {
  id: string;
  name: string;
  track_count: number | null;
  image_url: string | null;
  owner: string | null;
  url: string | null;
  kind: "playlist" | "saved_tracks" | "profile" | "library_playlist";
}

export type ImportSourceKind =
  "playlist" | "saved_tracks" | "profile" | "library_playlist" | "file";

export type ImportJobStatus = "fetching" | "matching" | "ready" | "failed";

export interface ImportJobSummary {
  id: string;
  service: ImportServiceId;
  source_kind: ImportSourceKind;
  source_ref: string;
  name: string;
  status: ImportJobStatus;
  error: string | null;
  source_url: string | null;
  playlist_id: number | null;
  created_at: number;
  updated_at: number;
  /** Tracks read so far while fetching. */
  fetched: number | null;
  total: number;
  /** Auto-matched plus confirmed. */
  matched: number;
  review: number;
  unmatched: number;
  skipped: number;
}

export type ImportItemStatus =
  "matched" | "review" | "unmatched" | "confirmed" | "skipped";

export type MatchReason =
  | "isrc"
  | "file_path"
  | "exact_title"
  | "similar_title"
  | "exact_artist"
  | "similar_artist"
  | "artist_mismatch"
  | "album"
  | "duration_close"
  | "duration_mismatch"
  | "version_mismatch"
  | "ambiguous";

export interface MatchCandidate {
  song_id: string;
  confidence: number;
  reasons: MatchReason[];
}

export interface ImportSourceTrack {
  title: string;
  artists: string[];
  album: string | null;
  duration_ms: number | null;
  isrc: string | null;
  location: string | null;
  external_id: string | null;
  external_url: string | null;
}

export interface ImportItem {
  position: number;
  source: ImportSourceTrack;
  status: ImportItemStatus;
  song_id: string | null;
  confidence: number;
  reasons: MatchReason[];
  candidates: MatchCandidate[];
}

export interface ImportSongBrief {
  id: string;
  name: string;
  artist: string;
  album: string;
  album_id: string;
  cover_url: string;
  /** Seconds. */
  duration: number;
  path: string;
}

export interface ImportJob extends ImportJobSummary {
  items: ImportItem[];
  songs: Record<string, ImportSongBrief>;
}

export type ImportItemAction =
  | { action: "accept"; position: number }
  | { action: "choose"; position: number; song_id: string }
  | { action: "skip"; position: number }
  | { action: "reset"; position: number };

export type CreateImportJob =
  | { url: string }
  | {
      service: Exclude<ImportServiceId, "file">;
      kind: ImportSourceKind;
      id: string;
      storefront?: string;
    }
  | { file_name: string; content: string };

export interface SpotifyDevice {
  id: string | null;
  name: string;
  type: string;
  is_active: boolean;
  is_restricted: boolean;
  volume_percent: number | null;
}

export interface SpotifyHandoffResult {
  started: boolean;
  resolved: number;
  unresolved_song_ids: string[];
}

export interface SavedImportPlaylist {
  playlist_id: number;
  added: number;
  created: boolean;
}

export const MAX_PLAYLIST_FILE_BYTES = 5 * 1024 * 1024;
export const MAX_SPOTIFY_HANDOFF_TRACKS = 50;
export const IMPORT_FILE_ACCEPT =
  ".m3u,.m3u8,.pls,.xspf,.csv,.tsv,.txt,.xml,audio/x-mpegurl,application/xspf+xml,text/csv,text/plain,application/xml";

export async function getImportServices(): Promise<ImportService[]> {
  const response = await api.get<ImportService[]>("/imports/services");
  return response.data;
}

export async function startSpotifyAuthorization(
  clientId: string,
  redirectUri: string,
): Promise<{ authorize_url: string; state: string }> {
  const response = await api.post<{ authorize_url: string; state: string }>(
    "/imports/services/spotify/authorize",
    { client_id: clientId.trim(), redirect_uri: redirectUri },
  );
  return response.data;
}

export async function completeSpotifyAuthorization(
  code: string,
  state: string,
): Promise<void> {
  await api.post("/imports/services/spotify/callback", { code, state });
}

export interface ImportServiceConfiguration {
  developer_token?: string;
  music_user_token?: string;
  storefront?: string;
  api_key?: string;
  profile_id?: string;
}

export async function configureImportService(
  service: Exclude<ImportServiceId, "spotify" | "file">,
  configuration: ImportServiceConfiguration,
): Promise<void> {
  await api.put(
    `/imports/services/${encodeURIComponent(service)}`,
    configuration,
    // Verifying credentials calls the service.
    { timeout: 30_000 },
  );
}

export async function disconnectImportService(
  service: Exclude<ImportServiceId, "file">,
): Promise<void> {
  await api.delete(`/imports/services/${encodeURIComponent(service)}`);
}

export async function getRemotePlaylists(
  service: Exclude<ImportServiceId, "file">,
): Promise<RemotePlaylist[]> {
  const response = await api.get<RemotePlaylist[]>(
    `/imports/services/${encodeURIComponent(service)}/playlists`,
    { timeout: 60_000 },
  );
  return response.data;
}

export async function createImportJob(
  source: CreateImportJob,
): Promise<ImportJobSummary> {
  const response = await api.post<ImportJobSummary>("/imports/jobs", source, {
    timeout: 30_000,
  });
  return response.data;
}

export async function getImportJobs(): Promise<ImportJobSummary[]> {
  const response = await api.get<ImportJobSummary[]>("/imports/jobs");
  return response.data;
}

export async function getImportJob(id: string): Promise<ImportJob> {
  const response = await api.get<ImportJob>(
    `/imports/jobs/${encodeURIComponent(id)}`,
  );
  return response.data;
}

export async function updateImportItems(
  id: string,
  actions: ImportItemAction[],
): Promise<ImportJob> {
  const response = await api.patch<ImportJob>(
    `/imports/jobs/${encodeURIComponent(id)}/items`,
    { actions },
  );
  return response.data;
}

export async function rematchImportJob(id: string): Promise<ImportJob> {
  const response = await api.post<ImportJob>(
    `/imports/jobs/${encodeURIComponent(id)}/rematch`,
  );
  return response.data;
}

export async function saveImportPlaylist(
  id: string,
  name?: string,
): Promise<SavedImportPlaylist> {
  const response = await api.post<SavedImportPlaylist>(
    `/imports/jobs/${encodeURIComponent(id)}/playlist`,
    { name: name?.trim() || undefined },
  );
  return response.data;
}

export async function deleteImportJob(id: string): Promise<void> {
  await api.delete(`/imports/jobs/${encodeURIComponent(id)}`);
}

export async function getSpotifyDevices(): Promise<SpotifyDevice[]> {
  const response = await api.get<SpotifyDevice[]>(
    "/imports/services/spotify/devices",
  );
  return response.data;
}

/** Asks one of the person's own Spotify devices to play these tracks. */
export async function handOffToSpotify(
  songIds: string[],
  deviceId?: string | null,
  positionMs = 0,
): Promise<SpotifyHandoffResult> {
  const ids = Array.from(new Set(songIds.filter(Boolean))).slice(
    0,
    MAX_SPOTIFY_HANDOFF_TRACKS,
  );
  const response = await api.post<SpotifyHandoffResult>(
    "/imports/services/spotify/handoff",
    {
      song_ids: ids,
      device_id: deviceId ?? undefined,
      position_ms: Math.max(0, Math.round(positionMs)),
    },
    { timeout: 45_000 },
  );
  return response.data;
}

/** The message a backend error carries, or a fallback. */
export function importErrorMessage(error: unknown, fallback: string): string {
  if (isApiError(error)) {
    const data = error.response?.data as { message?: unknown } | undefined;
    if (data && typeof data.message === "string" && data.message) {
      return data.message;
    }
  }
  return fallback;
}

/**
 * Decodes an uploaded playlist file. Apple Music's text export is UTF-16
 * with a byte-order mark; everything else is UTF-8.
 */
export function decodePlaylistFile(bytes: Uint8Array): string {
  if (bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xfe) {
    return new TextDecoder("utf-16le").decode(bytes.subarray(2));
  }
  if (bytes.length >= 2 && bytes[0] === 0xfe && bytes[1] === 0xff) {
    return new TextDecoder("utf-16be").decode(bytes.subarray(2));
  }
  return new TextDecoder("utf-8").decode(bytes).replace(/^﻿/, "");
}

export interface ParsedImportUrl {
  service: Exclude<ImportServiceId, "file">;
  kind: ImportSourceKind;
  id: string;
}

const SAFE_ID = /^(?!\.)[A-Za-z0-9._-]{1,128}$/;

/**
 * Recognises a pasted playlist link (mirrors the server's parser so the UI
 * can label it before submitting). Returns null for anything else.
 */
export function parseImportUrl(input: string): ParsedImportUrl | null {
  const text = input.trim();
  const spotifyUri = /^spotify:playlist:([^:]+)$/.exec(text);
  if (spotifyUri) {
    return SAFE_ID.test(spotifyUri[1]!)
      ? { service: "spotify", kind: "playlist", id: spotifyUri[1]! }
      : null;
  }
  let url: URL;
  try {
    url = new URL(/^[a-z]+:\/\//i.test(text) ? text : `https://${text}`);
  } catch {
    return null;
  }
  const host = url.hostname.toLowerCase().replace(/^www\./, "");
  const segments = url.pathname.split("/").filter(Boolean);
  if (host === "open.spotify.com" || host === "play.spotify.com") {
    const parts = segments.filter((part) => !part.startsWith("intl-"));
    if (parts[0] === "playlist" && parts[1] && SAFE_ID.test(parts[1])) {
      return { service: "spotify", kind: "playlist", id: parts[1] };
    }
    if (parts[0] === "collection" && parts[1] === "tracks") {
      return { service: "spotify", kind: "saved_tracks", id: "liked" };
    }
    return null;
  }
  if (host === "deezer.com") {
    const parts = segments.filter((part) => !/^[a-z]{2}$/.test(part));
    if (parts[0] === "playlist" && /^\d+$/.test(parts[1] ?? "")) {
      return { service: "deezer", kind: "playlist", id: parts[1]! };
    }
    if (parts[0] === "profile" && /^\d+$/.test(parts[1] ?? "")) {
      return { service: "deezer", kind: "profile", id: parts[1]! };
    }
    return null;
  }
  if (
    host === "music.youtube.com" ||
    host === "youtube.com" ||
    host === "m.youtube.com"
  ) {
    const list = url.searchParams.get("list");
    if (!list || !SAFE_ID.test(list) || ["LM", "WL", "LL"].includes(list)) {
      return null;
    }
    return { service: "youtube_music", kind: "playlist", id: list };
  }
  if (host === "music.apple.com" || host === "itunes.apple.com") {
    const id = segments.at(-1) ?? "";
    if (segments[1] !== "playlist" || !SAFE_ID.test(id)) return null;
    return {
      service: "apple_music",
      kind:
        segments[0] === "library" || id.startsWith("p.")
          ? "library_playlist"
          : "playlist",
      id,
    };
  }
  return null;
}

/** Share of a job that will be imported, 0..1. */
export function importMatchRate(job: ImportJobSummary): number {
  const considered = job.total - job.skipped;
  return considered > 0 ? job.matched / considered : 0;
}

/** Plain-language confidence band for a 0..1 score. */
export function confidenceLabel(confidence: number): "high" | "medium" | "low" {
  if (confidence >= 0.85) return "high";
  if (confidence >= 0.55) return "medium";
  return "low";
}

export const MATCH_REASON_LABELS: Record<MatchReason, string> = {
  isrc: "Same ISRC",
  file_path: "Same file",
  exact_title: "Same title",
  similar_title: "Similar title",
  exact_artist: "Same artist",
  similar_artist: "Similar artist",
  artist_mismatch: "Different artist",
  album: "Same album",
  duration_close: "Same length",
  duration_mismatch: "Different length",
  version_mismatch: "Different version",
  ambiguous: "Several close matches",
};
