import api from "../core/http";
import type {
  Album,
  Artist,
  AudioProperties,
  LibrarySong,
} from "../domain/types";

/**
 * TIDAL and Qobuz, connected per listener with credentials they supply.
 * Catalog ids are provider-scoped (`tidal:77646169`, `qobuz:0060254735180`)
 * and play through the same `/media/songs/{id}/stream` route as library
 * songs.
 */
export type StreamingProviderId = "tidal" | "qobuz";
export type StreamQuality =
  "hi_res_max" | "hi_res_96" | "lossless" | "high" | "low";
export type StreamingAccountStatus =
  "configured" | "pending" | "signed_in" | "expired" | "error";
export type StreamingItemKind = "album" | "track";
export type StreamingBrowseSection =
  "new_releases" | "favorite_albums" | "favorite_tracks";

export const STREAMING_PROVIDERS: readonly StreamingProviderId[] = [
  "tidal",
  "qobuz",
];

export const STREAMING_PROVIDER_NAMES: Record<StreamingProviderId, string> = {
  tidal: "TIDAL",
  qobuz: "Qobuz",
};

export interface StreamingAudioFormat {
  codec: string;
  lossless: boolean;
  bit_depth: number | null;
  sample_rate: number | null;
  bitrate_kbps: number | null;
  /** Badge text such as `24/96 FLAC`, `16/44.1 FLAC` or `AAC 320`. */
  label: string;
}

export interface StreamingArtistRef {
  id: string;
  name: string;
}

export interface StreamingAlbumRef {
  id: string;
  title: string;
  cover_url: string | null;
}

export interface StreamingTrack {
  id: string;
  provider: StreamingProviderId;
  title: string;
  version: string | null;
  artist: StreamingArtistRef;
  artists: StreamingArtistRef[];
  album: StreamingAlbumRef | null;
  track_number: number;
  disc_number: number;
  duration: number;
  explicit: boolean;
  isrc: string | null;
  format: StreamingAudioFormat | null;
  streamable: boolean;
  in_library: boolean;
}

export interface StreamingAlbum {
  id: string;
  provider: StreamingProviderId;
  title: string;
  version: string | null;
  artist: StreamingArtistRef;
  artists: StreamingArtistRef[];
  cover_url: string | null;
  release_date: string | null;
  track_count: number;
  duration: number;
  explicit: boolean;
  format: StreamingAudioFormat | null;
  label: string | null;
  genre: string | null;
  copyright: string | null;
  web_url: string | null;
  tracks: StreamingTrack[];
  in_library: boolean;
}

export interface StreamingArtistSummary {
  id: string;
  provider: StreamingProviderId;
  name: string;
  picture_url: string | null;
}

export interface StreamingArtist extends StreamingArtistSummary {
  albums: StreamingAlbum[];
  top_tracks: StreamingTrack[];
  web_url: string | null;
}

export interface StreamingSearchResults {
  albums: StreamingAlbum[];
  tracks: StreamingTrack[];
  artists: StreamingArtistSummary[];
}

export interface StreamingQualityOption {
  id: StreamQuality;
  label: string;
  detail: string;
}

export interface StreamingPendingSignIn {
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string | null;
  expires_at: number;
  interval: number;
}

export interface StreamingProviderStatus {
  id: StreamingProviderId;
  name: string;
  sign_in: "device_code" | "password";
  status: StreamingAccountStatus;
  status_message: string | null;
  account_name: string | null;
  plan: string | null;
  quality: StreamQuality;
  quality_options: StreamingQualityOption[];
  last_library_sync: number | null;
  credentials: {
    configured: boolean;
    client_id_hint: string | null;
    has_secret: boolean;
    uses_server_credentials: boolean;
  };
  server_credentials_available: boolean;
  pending: StreamingPendingSignIn | null;
  library: { albums: number; tracks: number };
  sandbox: boolean;
}

export interface StreamingProvidersResponse {
  sandbox: boolean;
  providers: StreamingProviderStatus[];
}

export interface StreamingSignInPoll {
  status: "pending" | "signed_in" | "expired" | "denied";
  interval: number;
  provider: StreamingProviderStatus | null;
}

export interface StreamingLibraryItem {
  id: string;
  provider: StreamingProviderId;
  kind: StreamingItemKind;
  title: string;
  artist_name: string;
  album_title: string | null;
  cover_url: string | null;
  release_date: string | null;
  duration: number;
  track_count: number;
  format_label: string | null;
  origin: "parson" | "sync";
  added_at: number;
  /** Snapshot taken when the item was added: a StreamingAlbum or StreamingTrack. */
  item: StreamingAlbum | StreamingTrack | null;
}

export interface StreamingLibraryPage {
  items: StreamingLibraryItem[];
  total: number;
}

export interface StreamingServiceSearch {
  provider: StreamingProviderId;
  name: string;
  results?: StreamingSearchResults;
  error?: string;
}

export interface StreamingSearchResponse {
  query: string;
  services: StreamingServiceSearch[];
}

export interface StreamingLibraryChange {
  in_library: boolean;
  service_updated: boolean;
}

export interface StreamingSyncReport {
  albums_added: number;
  albums_removed: number;
  tracks_added: number;
  tracks_removed: number;
  synced_at: number;
}

export interface StreamingStreamInfo {
  format: StreamingAudioFormat;
  requested: StreamQuality;
  sandbox: boolean;
}

const PROVIDER_ID_PATTERN = /^(tidal|qobuz):([A-Za-z0-9_-]{1,64})$/;

/** Splits a provider-scoped id; `null` for library ids. */
export function parseStreamingId(
  id: string | null | undefined,
): { provider: StreamingProviderId; rawId: string } | null {
  const match = PROVIDER_ID_PATTERN.exec(id ?? "");
  const [, provider, rawId] = match ?? [];
  if (!provider || !rawId) return null;
  return { provider: provider as StreamingProviderId, rawId };
}

export const isStreamingId = (id: string | null | undefined) =>
  parseStreamingId(id) !== null;

const entityPath = (kind: "albums" | "artists" | "tracks", id: string) => {
  const parsed = parseStreamingId(id);
  if (!parsed) throw new Error(`Not a streaming id: ${id}`);
  return `/providers/${parsed.provider}/${kind}/${encodeURIComponent(parsed.rawId)}`;
};

export async function getStreamingProviders(): Promise<StreamingProvidersResponse> {
  return (await api.get<StreamingProvidersResponse>("/providers")).data;
}

export async function saveStreamingCredentials(
  provider: StreamingProviderId,
  credentials:
    | { client_id: string; client_secret?: string | null }
    | { use_server_credentials: true },
): Promise<StreamingProviderStatus> {
  return (
    await api.put<StreamingProviderStatus>(
      `/providers/${provider}/credentials`,
      credentials,
    )
  ).data;
}

export async function startTidalSignIn(): Promise<StreamingPendingSignIn> {
  return (await api.post<StreamingPendingSignIn>("/providers/tidal/sign-in"))
    .data;
}

export async function pollTidalSignIn(): Promise<StreamingSignInPoll> {
  return (await api.post<StreamingSignInPoll>("/providers/tidal/sign-in/poll"))
    .data;
}

/** The password goes to the Parson server once and is never stored. */
export async function signInToQobuz(
  username: string,
  password: string,
): Promise<StreamingProviderStatus> {
  return (
    await api.post<StreamingProviderStatus>(
      "/providers/qobuz/sign-in",
      { username, password },
      { timeout: 30_000 },
    )
  ).data;
}

export async function setStreamingQuality(
  provider: StreamingProviderId,
  quality: StreamQuality,
): Promise<StreamingProviderStatus> {
  return (
    await api.patch<StreamingProviderStatus>(`/providers/${provider}`, {
      quality,
    })
  ).data;
}

export async function signOutOfStreamingProvider(
  provider: StreamingProviderId,
): Promise<StreamingProviderStatus> {
  return (
    await api.post<StreamingProviderStatus>(`/providers/${provider}/sign-out`)
  ).data;
}

/** Forgets credentials, session and every library item from the service. */
export async function removeStreamingProvider(
  provider: StreamingProviderId,
): Promise<StreamingProviderStatus> {
  return (await api.delete<StreamingProviderStatus>(`/providers/${provider}`))
    .data;
}

export async function syncStreamingLibrary(
  provider: StreamingProviderId,
): Promise<StreamingSyncReport> {
  return (
    await api.post<StreamingSyncReport>(
      `/providers/${provider}/library/sync`,
      undefined,
      { timeout: 120_000 },
    )
  ).data;
}

export async function searchStreaming(
  query: string,
  options: { providers?: StreamingProviderId[]; limit?: number } = {},
): Promise<StreamingSearchResponse> {
  return (
    await api.get<StreamingSearchResponse>("/providers/search", {
      params: {
        q: query,
        ...(options.providers?.length
          ? { providers: options.providers.join(",") }
          : {}),
        ...(options.limit ? { limit: options.limit } : {}),
      },
      timeout: 25_000,
    })
  ).data;
}

export async function getStreamingLibrary(
  options: {
    provider?: StreamingProviderId;
    kind?: StreamingItemKind;
    q?: string;
    sort?: "title" | "recent" | "artist";
    offset?: number;
    limit?: number;
  } = {},
): Promise<StreamingLibraryPage> {
  const params = Object.fromEntries(
    Object.entries(options).filter(
      ([, value]) => value !== undefined && value !== "",
    ),
  );
  return (await api.get<StreamingLibraryPage>("/providers/library", { params }))
    .data;
}

export async function getStreamingAlbum(id: string): Promise<StreamingAlbum> {
  return (await api.get<StreamingAlbum>(entityPath("albums", id))).data;
}

export async function getStreamingArtist(id: string): Promise<StreamingArtist> {
  return (await api.get<StreamingArtist>(entityPath("artists", id))).data;
}

export async function getStreamInfo(
  trackId: string,
): Promise<StreamingStreamInfo> {
  return (
    await api.get<StreamingStreamInfo>(
      `${entityPath("tracks", trackId)}/stream-info`,
    )
  ).data;
}

export async function browseStreaming(
  provider: StreamingProviderId,
  section: StreamingBrowseSection,
  options: { offset?: number; limit?: number } = {},
): Promise<{
  albums: StreamingAlbum[];
  tracks: StreamingTrack[];
  total: number | null;
}> {
  return (
    await api.get(`/providers/${provider}/browse/${section}`, {
      params: options,
    })
  ).data;
}

const libraryPath = (kind: StreamingItemKind, id: string) => {
  const parsed = parseStreamingId(id);
  if (!parsed) throw new Error(`Not a streaming id: ${id}`);
  return `/providers/${parsed.provider}/library/${kind}/${encodeURIComponent(parsed.rawId)}`;
};

/** Adds to the Parson library and favorites it on the service. */
export async function addToStreamingLibrary(
  kind: StreamingItemKind,
  id: string,
): Promise<StreamingLibraryChange> {
  return (await api.put<StreamingLibraryChange>(libraryPath(kind, id))).data;
}

export async function removeFromStreamingLibrary(
  kind: StreamingItemKind,
  id: string,
): Promise<StreamingLibraryChange> {
  return (await api.delete<StreamingLibraryChange>(libraryPath(kind, id))).data;
}

/** Audio properties for the player's quality display. */
export function streamingAudio(
  format: StreamingAudioFormat | null | undefined,
): AudioProperties | null {
  if (!format) return null;
  return {
    codec: format.codec,
    container: null,
    sample_rate: format.sample_rate,
    bit_depth: format.bit_depth,
    channels: 2,
    bitrate_kbps: format.bitrate_kbps,
    lossless: format.lossless,
  };
}

const displayTitle = (title: string, version: string | null) =>
  version && !title.toLowerCase().includes(version.toLowerCase())
    ? `${title} (${version})`
    : title;

/** A streaming album in the shape the player and album views use. */
export function streamingAlbumToAlbum(
  album: StreamingAlbum,
): Album & { artist_object: Artist } {
  return {
    id: album.id,
    name: displayTitle(album.title, album.version),
    cover_url: album.cover_url ?? "",
    songs: album.tracks.map((track) => streamingTrackToSong(track, album)),
    first_release_date: album.release_date ?? "",
    musicbrainz_id: "",
    wikidata_id: null,
    primary_type: "album",
    description: "",
    contributing_artists: album.artists.map((artist) => artist.name),
    contributing_artists_ids: album.artists.map((artist) => artist.id),
    label: album.label,
    artist_object: {
      id: album.artist.id,
      name: album.artist.name,
      icon_url: "",
      followers: 0,
      albums: [],
      featured_on_album_ids: [],
      description: "",
    },
  };
}

/** A streaming track as a playable song (its id streams through Parson). */
export function streamingTrackToSong(
  track: StreamingTrack,
  album?: Pick<StreamingAlbum, "id" | "title" | "cover_url" | "artist">,
): LibrarySong {
  const albumRef = album
    ? { id: album.id, title: album.title, cover_url: album.cover_url }
    : track.album;
  const albumArtist = album?.artist ?? track.artist;
  const featured = track.artists.filter(
    (artist) => artist.id !== track.artist.id,
  );
  return {
    id: track.id,
    name: displayTitle(track.title, track.version),
    artist: track.artist.name,
    contributing_artists: featured.map((artist) => artist.name),
    contributing_artist_ids: featured.map((artist) => artist.id),
    track_number: track.track_number,
    path: "",
    duration: track.duration,
    isrc: track.isrc,
    audio: streamingAudio(track.format),
    artist_object: {
      id: albumArtist.id,
      name: albumArtist.name,
      icon_url: "",
      followers: 0,
      albums: [],
      featured_on_album_ids: [],
      description: "",
    },
    album_object: {
      id: albumRef?.id ?? "",
      name: albumRef?.title ?? "",
      cover_url: albumRef?.cover_url ?? "",
      songs: [],
      first_release_date: "",
      musicbrainz_id: "",
      wikidata_id: null,
      primary_type: "album",
      description: "",
      contributing_artists: [],
      contributing_artists_ids: [],
    },
  };
}

/** `1792870534` -> a short local time, or "Never". */
export function formatLastSync(
  seconds: number | null | undefined,
  now: Date = new Date(),
): string {
  if (!seconds) return "Never";
  const date = new Date(seconds * 1000);
  const sameDay = date.toDateString() === now.toDateString();
  return sameDay
    ? date.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      })
    : date.toLocaleString([], {
        day: "numeric",
        month: "short",
        hour: "2-digit",
        minute: "2-digit",
      });
}
