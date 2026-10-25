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
  /** Present when the format was read from the stream's own bytes. */
  channels?: number | null;
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

/** A service playlist, read-only in Parson. */
export interface StreamingPlaylist {
  id: string;
  provider: StreamingProviderId;
  title: string;
  description: string | null;
  owner: string | null;
  /** The listener made it, rather than following it. */
  own: boolean;
  track_count: number;
  duration: number;
  cover_url: string | null;
  updated_at: number | null;
  web_url: string | null;
  /** Present on a single playlist; empty in lists. */
  tracks: StreamingTrack[];
  /** When the listener last saved a copy into Parson. */
  saved_copy_at: number | null;
}

/** A read-only copy of a service playlist kept in Parson. */
export interface StreamingPlaylistCopy {
  id: string;
  provider: StreamingProviderId;
  title: string;
  owner: string | null;
  cover_url: string | null;
  track_count: number;
  duration: number;
  saved_at: number;
  /** The saved tracks, on a single copy. */
  playlist?: Omit<StreamingPlaylist, "saved_copy_at">;
}

/** Where the server-wide credentials came from. */
export type StreamingCredentialSource = "environment" | "settings";

/** Server-wide credentials as admins see them: never the values. */
export interface StreamingServerCredential {
  provider: StreamingProviderId;
  name: string;
  configured: boolean;
  source: StreamingCredentialSource | null;
  client_id_hint: string | null;
  has_secret: boolean;
  updated_at: number | null;
}

/** What a stream actually delivered, read from its bytes by the server. */
export interface StreamingDelivery {
  track_id: string;
  requested: StreamQuality;
  /** What the service said it would send. */
  announced: StreamingAudioFormat;
  /** What arrived; `null` if the bytes were not a known format. */
  delivered: StreamingAudioFormat | null;
  container: "flac" | "mp4" | "wav" | "mp3" | "adts" | null;
  /** The first audio frame was parsed and agrees with the header. */
  frame_verified: boolean;
  matches_announced: boolean;
  sandbox: boolean;
  at: number;
}

export interface StreamingCheckStep {
  id: "session" | "favorites" | "stream";
  ok: boolean;
  detail: string;
}

/** "Check connection": sign-in, favorites sync and a real stream read. */
export interface StreamingConnectionCheck {
  provider: StreamingProviderId;
  ok: boolean;
  sandbox: boolean;
  account_name: string | null;
  plan: string | null;
  steps: StreamingCheckStep[];
  sync: StreamingSyncReport | null;
  track: StreamingTrack | null;
  delivery: StreamingDelivery | null;
  checked_at: number;
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

const entityPath = (
  kind: "albums" | "artists" | "tracks" | "playlists",
  id: string,
) => {
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

/** Admins only. */
export async function getStreamingServerCredentials(): Promise<
  StreamingServerCredential[]
> {
  return (
    await api.get<{ providers: StreamingServerCredential[] }>(
      "/providers/server-credentials",
    )
  ).data.providers;
}

/**
 * Admins only: the TIDAL client or Qobuz app every listener on this server
 * signs in through, so listeners never need credentials of their own.
 */
export async function saveStreamingServerCredentials(
  provider: StreamingProviderId,
  credentials: { client_id: string; client_secret?: string | null },
): Promise<StreamingServerCredential[]> {
  return (
    await api.put<{ providers: StreamingServerCredential[] }>(
      `/providers/${provider}/server-credentials`,
      credentials,
    )
  ).data.providers;
}

export async function removeStreamingServerCredentials(
  provider: StreamingProviderId,
): Promise<StreamingServerCredential[]> {
  return (
    await api.delete<{ providers: StreamingServerCredential[] }>(
      `/providers/${provider}/server-credentials`,
    )
  ).data.providers;
}

/** Signs in, syncs favorites and reads a real stream's delivered format. */
export async function checkStreamingConnection(
  provider: StreamingProviderId,
  trackId?: string,
): Promise<StreamingConnectionCheck> {
  return (
    await api.post<StreamingConnectionCheck>(
      `/providers/${provider}/check`,
      trackId ? { track_id: trackId } : {},
      { timeout: 120_000 },
    )
  ).data;
}

/**
 * The format the latest stream of this track delivered, or `null` before
 * its first bytes reached the player.
 */
export async function getStreamDelivery(
  trackId: string,
): Promise<StreamingDelivery | null> {
  try {
    return (
      await api.get<StreamingDelivery>(
        `${entityPath("tracks", trackId)}/delivery`,
      )
    ).data;
  } catch (error) {
    const status = (error as { response?: { status?: number } }).response
      ?.status;
    if (status === 404) return null;
    throw error;
  }
}

/** "24/192 FLAC, stereo" style text for a delivered format. */
export function describeDelivery(delivery: StreamingDelivery): string {
  const format = delivery.delivered;
  if (!format) return "Format not readable";
  const channels =
    format.channels === 1
      ? ", mono"
      : format.channels === 2
        ? ", stereo"
        : format.channels
          ? `, ${format.channels} channels`
          : "";
  return `${format.label}${channels}`;
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

export async function getStreamingPlaylists(
  provider: StreamingProviderId,
  options: { offset?: number; limit?: number } = {},
): Promise<{ playlists: StreamingPlaylist[]; total: number | null }> {
  return (
    await api.get<{ playlists: StreamingPlaylist[]; total: number | null }>(
      `/providers/${provider}/playlists`,
      { params: options },
    )
  ).data;
}

export async function getStreamingPlaylist(
  id: string,
): Promise<StreamingPlaylist> {
  return (
    await api.get<StreamingPlaylist>(entityPath("playlists", id), {
      timeout: 30_000,
    })
  ).data;
}

export async function getStreamingPlaylistCopies(): Promise<
  StreamingPlaylistCopy[]
> {
  return (
    await api.get<{ copies: StreamingPlaylistCopy[] }>(
      "/providers/playlist-copies",
    )
  ).data.copies;
}

export async function getStreamingPlaylistCopy(
  id: string,
): Promise<StreamingPlaylistCopy> {
  return (
    await api.get<StreamingPlaylistCopy>(`${entityPath("playlists", id)}/copy`)
  ).data;
}

/** Saves (or refreshes) a copy in Parson; the service playlist is unchanged. */
export async function saveStreamingPlaylistCopy(
  id: string,
): Promise<StreamingPlaylistCopy> {
  return (
    await api.put<StreamingPlaylistCopy>(
      `${entityPath("playlists", id)}/copy`,
      undefined,
      { timeout: 30_000 },
    )
  ).data;
}

export async function removeStreamingPlaylistCopy(id: string): Promise<void> {
  await api.delete(`${entityPath("playlists", id)}/copy`);
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
