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
/** Editorial lists (`new_releases` ... `press_awards`) and favorites. */
export type StreamingEditorialSection =
  "new_releases" | "popular" | "editors_picks" | "press_awards";
export type StreamingBrowseSection =
  StreamingEditorialSection | "favorite_albums" | "favorite_tracks";

/** An editorial list a service publishes, in its own app's order. */
export interface StreamingBrowseSectionInfo {
  id: StreamingEditorialSection;
  label: string;
  /** Whether the list can be narrowed to one genre (Qobuz). */
  by_genre: boolean;
}

/** A service genre; `id` is the service's own key. */
export interface StreamingGenre {
  id: string;
  provider: StreamingProviderId;
  name: string;
  image_url?: string | null;
}

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
  /** Who played, wrote and produced it; on album pages, when the service says. */
  credits?: StreamingCredit[];
}

/** One credited role, such as `Producer: A, B`. */
export interface StreamingCredit {
  role: string;
  names: string[];
}

/** A service's album review or artist biography, as plain paragraphs. */
export interface StreamingEditorialText {
  text: string;
  /** Who wrote it, when the service says. */
  source: string | null;
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
  /** The service's review or description, on album pages. */
  review?: StreamingEditorialText | null;
  /** Press awards such as `Qobuzissime`. */
  awards?: string[];
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
  bio?: StreamingEditorialText | null;
  /** Artists the service considers similar. */
  similar?: StreamingArtistSummary[];
}

export type StreamingRadioSeed = "track" | "artist" | "album";

/**
 * A radio: TIDAL's own track or artist radio (`service`), or for Qobuz,
 * which publishes none, Parson's mix of the artist's and similar artists'
 * top tracks (`similar_artists`).
 */
export interface StreamingRadio {
  tracks: StreamingTrack[];
  origin: "service" | "similar_artists";
}

/** How a radio's tracks were chosen, for the UI. */
export function describeRadio(
  provider: StreamingProviderId,
  radio: Pick<StreamingRadio, "origin">,
): string {
  return radio.origin === "service"
    ? `${STREAMING_PROVIDER_NAMES[provider]} radio`
    : `Parson radio from ${STREAMING_PROVIDER_NAMES[provider]} similar artists`;
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
  browse_sections: StreamingBrowseSectionInfo[];
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
  /** The last "Check connection" on this account; null until one runs. */
  last_check?: StreamingAccountCheck | null;
}

/**
 * What the last "Check connection" proved about one account. `verified`
 * means the real service (not the sandbox) sent the whole track, lossless,
 * in the format it announced.
 */
export interface StreamingAccountCheck {
  checked_at: number;
  ok: boolean;
  sandbox: boolean;
  verified: boolean;
  failed_step: "session" | "favorites" | "stream" | null;
  detail: string;
  requested: StreamQuality | null;
  announced: string | null;
  delivered: string | null;
  lossless: boolean | null;
  matches_announced: boolean | null;
  full_length: boolean | null;
  delivered_seconds: number | null;
  track_seconds: number | null;
  track_title: string | null;
}

export type AccountProofTone = "verified" | "partial" | "failed" | "unproven";

export interface AccountProof {
  tone: AccountProofTone;
  /** Short line for a services list, e.g. "Verified: full-length 24/192 FLAC". */
  label: string;
  /** One sentence saying exactly what was and was not proven. */
  detail: string;
}

/**
 * Says honestly whether a signed-in account has been proven to stream
 * whole tracks in lossless quality, from its last connection check. A
 * signed-in account that was never checked is "unproven", never assumed
 * to work.
 */
export function accountProof(
  provider: Pick<StreamingProviderStatus, "name" | "status" | "last_check">,
  now: Date = new Date(),
): AccountProof | null {
  if (provider.status !== "signed_in" && !provider.last_check) return null;
  const check = provider.last_check;
  if (!check) {
    return {
      tone: "unproven",
      label: "Not checked yet",
      detail: `Run Check connection to prove ${provider.name} sends this account full-length lossless streams.`,
    };
  }
  const when = formatLastSync(check.checked_at, now);
  if (check.sandbox) {
    return {
      tone: "unproven",
      label: "Sandbox only",
      detail: `Checked against recorded ${provider.name} replies (${when}), not the real service. This proves Parson's side only.`,
    };
  }
  if (check.verified) {
    return {
      tone: "verified",
      label: `Verified: full-length ${check.delivered ?? "lossless"}`,
      detail: `${when}: ${provider.name} sent this account the whole of ${check.track_title ? `"${check.track_title}"` : "a track"} as ${check.delivered}, the format it announced.`,
    };
  }
  if (check.ok) {
    // Worked, but not proof of lossless: a lossy plan or an unknown length.
    const reason =
      check.lossless === false
        ? `it was lossy (${check.delivered ?? "unknown format"})`
        : check.full_length == null
          ? "the stream did not state its length"
          : `it was ${check.delivered ?? "an unread format"}`;
    return {
      tone: "partial",
      label: `Plays ${check.delivered ?? "audio"}`,
      detail: `${when}: streaming works, but ${reason}, so lossless playback is not proven.`,
    };
  }
  const failed =
    check.failed_step === "stream" && check.full_length === false
      ? "Preview only"
      : check.failed_step === "session"
        ? "Sign-in failed"
        : check.failed_step === "favorites"
          ? "Favorites failed"
          : "Stream failed";
  return {
    tone: "failed",
    label: `Last check: ${failed}`,
    detail: `${when}: ${check.detail}`,
  };
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

/**
 * A copy of a service playlist kept in Parson, editable there (the service
 * playlist never changes), or a playlist made in Parson from a radio or a
 * selection of one service's tracks.
 */
export interface StreamingPlaylistCopy {
  id: string;
  provider: StreamingProviderId;
  title: string;
  owner: string | null;
  cover_url: string | null;
  track_count: number;
  duration: number;
  saved_at: number;
  /** When the listener last changed it in Parson. */
  edited_at?: number | null;
  /** Made in Parson; there is no service playlist to save again. */
  made_in_parson?: boolean;
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
  /**
   * Codec, and bit depth and rate (lossless) or bitrate within 10% (lossy),
   * agree with what the service announced.
   */
  matches_announced: boolean;
  /** What disagrees, e.g. `255 kbps, not 320`. Empty when it matches. */
  mismatches?: string[];
  /** The player's id for the play this report belongs to. */
  play?: string | null;
  /** Larger is newer. */
  seq?: number;
  /**
   * The delivered stream's length from its own header or DASH timeline;
   * `null` when the stream does not state it (MP3, ADTS).
   */
  delivered_seconds?: number | null;
  /** The catalog length, when the server knew it. */
  track_seconds?: number | null;
  /**
   * The whole track arrived rather than a preview. `null` when a length is
   * unknown, and always in the sandbox (30-second renders by design).
   */
  full_length?: boolean | null;
  sandbox: boolean;
  /**
   * The sandbox's 16/44.1 placeholder tone (server without ffmpeg). Other
   * sandbox streams are rendered in the announced format and checked like
   * live ones.
   */
  tone?: boolean;
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

const PLAY_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;

/** Whether `value` can name one play of a stream (`?play=`). */
export function isPlayId(value: string | null | undefined): value is string {
  return Boolean(value && PLAY_ID_PATTERN.test(value));
}

let playCounter = 0;

/**
 * A fresh id for one play of a streaming track. The player adds it to the
 * stream URL as `play`, then asks for that play's delivery report, so the
 * format it shows is the stream it is playing and never an earlier one.
 */
export function newPlayId(): string {
  playCounter = (playCounter + 1) % 1_000_000;
  const random = Math.random().toString(36).slice(2, 10);
  return `p${Date.now().toString(36)}${playCounter.toString(36)}${random}`;
}

/** The `play` id in a stream URL, if it has one. */
export function playIdFromStreamUrl(url: string | null | undefined) {
  if (!url) return null;
  const match = /[?&]play=([^&#]*)/.exec(url);
  const value = match?.[1] ? decodeURIComponent(match[1]) : null;
  return isPlayId(value) ? value : null;
}

/**
 * The format a stream of this track delivered, or `null` before its first
 * bytes reached the server. With `play`, only that play's report: never an
 * earlier stream of the same track at another quality.
 */
export async function getStreamDelivery(
  trackId: string,
  play?: string | null,
): Promise<StreamingDelivery | null> {
  const path = `${entityPath("tracks", trackId)}/delivery`;
  try {
    return (
      isPlayId(play)
        ? await api.get<StreamingDelivery>(path, { params: { play } })
        : await api.get<StreamingDelivery>(path)
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

/** A TIDAL browser sign-in (authorization code with PKCE). */
export interface StreamingBrowserSignIn {
  /** Where the listener approves; TIDAL then returns to `redirect_uri`. */
  authorize_url: string;
  redirect_uri: string;
  expires_at: number;
  /** Sandbox sign-ins skip TIDAL's page and come straight back. */
  sandbox: boolean;
}

/** The web app page TIDAL sends a browser sign-in back to. */
export const TIDAL_CALLBACK_PATH = "/settings/services/tidal-callback";

/** The redirect URI to register with TIDAL for a Parson server address. */
export function tidalRedirectUri(origin: string): string {
  return `${origin.replace(/\/+$/, "")}${TIDAL_CALLBACK_PATH}`;
}

/**
 * Starts a browser sign-in, the flow TIDAL gives clients registered on its
 * developer dashboard. Send the listener to `authorize_url`.
 */
export async function startTidalBrowserSignIn(
  redirectUri: string,
): Promise<StreamingBrowserSignIn> {
  return (
    await api.post<StreamingBrowserSignIn>("/providers/tidal/sign-in/browser", {
      redirect_uri: redirectUri,
    })
  ).data;
}

/** Finishes a browser sign-in with what TIDAL put in the callback URL. */
export async function finishTidalBrowserSignIn(
  state: string,
  code: string,
): Promise<StreamingProviderStatus> {
  return (
    await api.post<StreamingProviderStatus>(
      "/providers/tidal/sign-in/browser/finish",
      { state, code },
      { timeout: 30_000 },
    )
  ).data;
}

/**
 * Reads TIDAL's answer from a callback URL's query string: the code and
 * state on approval, or TIDAL's error.
 */
export function readTidalCallback(
  search: string,
): { ok: true; code: string; state: string } | { ok: false; error: string } {
  const params = new URLSearchParams(search);
  const error = params.get("error");
  if (error) {
    const description = params.get("error_description");
    return {
      ok: false,
      error:
        error === "access_denied"
          ? "Sign-in was declined on TIDAL."
          : description || `TIDAL answered ${error}.`,
    };
  }
  const code = params.get("code");
  const state = params.get("state");
  if (!code || !state) {
    return { ok: false, error: "TIDAL sent back no sign-in code." };
  }
  return { ok: true, code, state };
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

/**
 * Saves (or refreshes) a copy in Parson; the service playlist is unchanged.
 * Refreshing a copy edited in Parson fails with 409 `provider_copy_edited`
 * unless `replaceEdits` is set.
 */
export async function saveStreamingPlaylistCopy(
  id: string,
  options: { replaceEdits?: boolean } = {},
): Promise<StreamingPlaylistCopy> {
  return (
    await api.put<StreamingPlaylistCopy>(
      `${entityPath("playlists", id)}/copy`,
      undefined,
      {
        timeout: 30_000,
        params: options.replaceEdits ? { replace_edits: true } : undefined,
      },
    )
  ).data;
}

/** Changes to a copy; fields left out stay as they are. */
export interface StreamingPlaylistCopyEdit {
  title?: string;
  description?: string;
  /** The tracks in their new order. Ids not yet in the copy are added. */
  track_ids?: string[];
}

/** Renames, reorders, removes or adds tracks, in Parson only. */
export async function editStreamingPlaylistCopy(
  id: string,
  edit: StreamingPlaylistCopyEdit,
): Promise<StreamingPlaylistCopy> {
  return (
    await api.patch<StreamingPlaylistCopy>(
      `${entityPath("playlists", id)}/copy`,
      edit,
      { timeout: 30_000 },
    )
  ).data;
}

/** A playlist made in Parson from one service's tracks, e.g. a radio. */
export async function createStreamingPlaylistCopy(
  provider: StreamingProviderId,
  playlist: { title: string; description?: string; track_ids: string[] },
): Promise<StreamingPlaylistCopy> {
  return (
    await api.post<StreamingPlaylistCopy>(
      `/providers/${provider}/playlist-copies`,
      playlist,
      { timeout: 30_000 },
    )
  ).data;
}

/** Starts a radio from a TIDAL or Qobuz track, artist or album. */
export async function getStreamingRadio(
  seed: StreamingRadioSeed,
  id: string,
): Promise<StreamingRadio> {
  const parsed = parseStreamingId(id);
  if (!parsed) throw new Error(`Not a streaming id: ${id}`);
  return (
    await api.get<StreamingRadio>(
      `/providers/${parsed.provider}/radio/${seed}/${encodeURIComponent(parsed.rawId)}`,
      { timeout: 30_000 },
    )
  ).data;
}

/** Moves one track up or down in an ordered id list (for copy editors). */
export function moveTrackId(
  ids: readonly string[],
  index: number,
  offset: -1 | 1,
): string[] {
  const target = index + offset;
  if (index < 0 || index >= ids.length || target < 0 || target >= ids.length) {
    return [...ids];
  }
  const next = [...ids];
  const [moved] = next.splice(index, 1);
  if (moved !== undefined) next.splice(target, 0, moved);
  return next;
}

/** Groups credits for display, main artists first, at most `limit` roles. */
export function creditLines(
  credits: readonly StreamingCredit[] | undefined,
  limit = 6,
): string[] {
  const order = (role: string) =>
    /main ?artist|performer/i.test(role)
      ? 0
      : /composer|lyricist|writer/i.test(role)
        ? 1
        : 2;
  return [...(credits ?? [])]
    .filter((credit) => credit.names.length > 0)
    .sort((a, b) => order(a.role) - order(b.role))
    .slice(0, limit)
    .map((credit) => `${prettyRole(credit.role)}: ${credit.names.join(", ")}`);
}

/** `MainArtist` -> `Main artist`, `Mixing Engineer` -> `Mixing engineer`. */
export function prettyRole(role: string): string {
  const spaced = role
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim()
    .toLowerCase();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
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

export async function getStreamingGenres(
  provider: StreamingProviderId,
): Promise<StreamingGenre[]> {
  return (
    await api.get<{ genres: StreamingGenre[] }>(`/providers/${provider}/genres`)
  ).data.genres;
}

export interface StreamingGenreAlbums {
  genre: StreamingGenre;
  albums: StreamingAlbum[];
  total: number | null;
}

/** One genre's albums, optionally from one editorial list (Qobuz). */
export async function getStreamingGenreAlbums(
  provider: StreamingProviderId,
  genreId: string,
  options: {
    section?: StreamingEditorialSection;
    offset?: number;
    limit?: number;
  } = {},
): Promise<StreamingGenreAlbums> {
  return (
    await api.get<StreamingGenreAlbums>(
      `/providers/${provider}/genres/${encodeURIComponent(genreId)}/albums`,
      { params: options },
    )
  ).data;
}

/** Where an album in the unified catalog lives. */
export type CatalogSource = "local" | StreamingProviderId;
export type CatalogQuality = "hi_res" | "lossless" | "lossy";

/** One album tile: local files or a streaming library album. */
export interface CatalogAlbum {
  key: string;
  /** A library album id, or a provider-scoped id such as `qobuz:...`. */
  id: string;
  name: string;
  artist_id: string;
  artist_name: string;
  /** A library cover path, or an artwork URL for streaming albums. */
  cover: string;
  year: number | null;
  genres: string[];
  source: CatalogSource;
  format_label: string | null;
  quality: CatalogQuality | null;
  /** The genres came from another copy of the album (TIDAL has none). */
  genre_inferred?: boolean;
}

export interface CatalogFacet<T> {
  value: T;
  count: number;
}

export interface CatalogFacets {
  sources: CatalogFacet<CatalogSource>[];
  genres: CatalogFacet<string>[];
  decades: CatalogFacet<number>[];
  qualities: CatalogFacet<CatalogQuality>[];
}

export interface CatalogAlbumPage {
  albums: CatalogAlbum[];
  total: number;
  /** Each facet counts albums matching every other filter. */
  facets: CatalogFacets;
  local_ready: boolean;
}

/** Album order: artist (default), title, newest, oldest or best quality first. */
export type CatalogSort = "artist" | "title" | "newest" | "oldest" | "quality";

export interface CatalogFilter {
  source?: CatalogSource;
  genre?: string;
  /** A year ending in 0, e.g. 1990. */
  decade?: number;
  quality?: CatalogQuality;
  q?: string;
  sort?: CatalogSort;
}

/**
 * Local and streaming albums in one catalog, filtered by source, genre,
 * decade and quality together (Roon's Focus).
 */
export async function getCatalogAlbums(
  filter: CatalogFilter = {},
  page: { offset?: number; limit?: number } = {},
): Promise<CatalogAlbumPage> {
  const params = Object.fromEntries(
    Object.entries({ ...filter, ...page }).filter(
      ([, value]) => value !== undefined && value !== "",
    ),
  );
  return (
    await api.get<CatalogAlbumPage>("/providers/catalog/albums", { params })
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

/**
 * Which copy plays when an album is in the local library and on TIDAL or
 * Qobuz. `highest_quality` picks the highest bit depth and sample rate
 * (local files win ties); the others put one source first.
 */
export type StreamingSourcePreference =
  "highest_quality" | "local" | StreamingProviderId;

export const SOURCE_PREFERENCES: readonly StreamingSourcePreference[] = [
  "highest_quality",
  "local",
  "qobuz",
  "tidal",
];

export const SOURCE_PREFERENCE_LABELS: Record<
  StreamingSourcePreference,
  string
> = {
  highest_quality: "Highest quality",
  local: "My files first",
  qobuz: "Qobuz first",
  tidal: "TIDAL first",
};

export async function getSourcePreference(): Promise<StreamingSourcePreference> {
  return (
    await api.get<{ source: StreamingSourcePreference }>(
      "/providers/preferences/source",
    )
  ).data.source;
}

export async function setSourcePreference(
  source: StreamingSourcePreference,
): Promise<StreamingSourcePreference> {
  return (
    await api.put<{ source: StreamingSourcePreference }>(
      "/providers/preferences/source",
      { source },
    )
  ).data.source;
}

export type AlbumVersionSource = "local" | StreamingProviderId;

/** One place an album lives, with the best format it is offered in. */
export interface AlbumVersion {
  id: string;
  title: string;
  source: AlbumVersionSource;
  format: Pick<
    StreamingAudioFormat,
    "lossless" | "bit_depth" | "sample_rate" | "bitrate_kbps" | "label"
  > | null;
}

export interface RankedAlbumVersion extends AlbumVersion {
  /** The copy that plays under the listener's preference. */
  preferred: boolean;
  /** Why this copy is first, for the preferred one only. */
  reason: string | null;
}

/**
 * A sortable quality score: any lossless copy beats any lossy one, then bit
 * depth, then sample rate; lossy copies compare by bitrate. Unknown formats
 * score lowest.
 */
export function formatScore(format: AlbumVersion["format"]): number {
  if (!format) return -1;
  if (format.lossless) {
    return (
      1_000_000_000 +
      (format.bit_depth ?? 16) * 1_000_000 +
      (format.sample_rate ?? 44_100)
    );
  }
  return format.bitrate_kbps ?? 0;
}

const SOURCE_NAMES: Record<AlbumVersionSource, string> = {
  local: "your files",
  qobuz: "Qobuz",
  tidal: "TIDAL",
};

const TIE_ORDER: Record<AlbumVersionSource, number> = {
  local: 0,
  qobuz: 1,
  tidal: 2,
};

/**
 * Orders an album's copies for a preference and marks the one that plays.
 * Ties go to local files (no network), then Qobuz, then TIDAL.
 */
export function rankAlbumVersions(
  versions: readonly AlbumVersion[],
  preference: StreamingSourcePreference,
): RankedAlbumVersion[] {
  const favored = preference === "highest_quality" ? null : preference;
  const ordered = [...versions].sort((a, b) => {
    if (favored && (a.source === favored) !== (b.source === favored)) {
      return a.source === favored ? -1 : 1;
    }
    const byQuality = formatScore(b.format) - formatScore(a.format);
    if (byQuality !== 0) return byQuality;
    return TIE_ORDER[a.source] - TIE_ORDER[b.source];
  });
  return ordered.map((version, index) => {
    if (index > 0) return { ...version, preferred: false, reason: null };
    const label = version.format?.label;
    let reason: string;
    if (favored && version.source === favored) {
      reason = `${SOURCE_PREFERENCE_LABELS[preference]} in your settings`;
    } else if (ordered.length === 1) {
      reason = "Only copy";
    } else if (favored) {
      reason = `Not in ${SOURCE_NAMES[favored]}; best of the rest`;
    } else if (
      formatScore(version.format) > formatScore(ordered[1]?.format ?? null)
    ) {
      reason = label ? `Highest quality: ${label}` : "Highest quality";
    } else {
      reason = "Same quality; local files and Qobuz win ties";
    }
    return { ...version, preferred: true, reason };
  });
}

/**
 * The best format a local album's files are in, from the technical
 * properties the indexer read, as a version format.
 */
export function localAlbumFormat(
  songs: readonly { audio?: AudioProperties | null }[],
): AlbumVersion["format"] {
  let best: AlbumVersion["format"] = null;
  for (const song of songs) {
    const audio = song.audio;
    if (!audio?.codec) continue;
    const lossless = audio.lossless === true;
    const codec = audio.codec.toUpperCase();
    const label = lossless
      ? `${audio.bit_depth ?? 16}/${formatKhz(audio.sample_rate ?? 44_100)} ${codec}`
      : `${codec} ${audio.bitrate_kbps ?? ""}`.trim();
    const format = {
      lossless,
      bit_depth: audio.bit_depth,
      sample_rate: audio.sample_rate,
      bitrate_kbps: audio.bitrate_kbps,
      label,
    };
    if (formatScore(format) > formatScore(best)) best = format;
  }
  return best;
}

function formatKhz(rate: number): string {
  const khz = rate / 1000;
  return Number.isInteger(khz) ? String(khz) : khz.toFixed(1);
}
