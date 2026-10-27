import api, { getApiBaseURL } from "../core/http";

/**
 * Live internet radio: the radio-browser.info community directory, My Live
 * Radio, stations added by URL, and what each station is playing.
 *
 * Stations play like songs: their song id is `radio:<station id>` and they
 * stream through `/media/songs/radio:<id>/stream`, so every player and zone
 * plays them through the Parson server.
 */

export type LiveStationSource = "directory" | "custom";

/** How good a station sounds. Lossy radio is never called lossless. */
export type LiveQualityTier =
  "unknown" | "low" | "standard" | "high" | "lossless";

export interface LiveQuality {
  tier: LiveQualityTier;
  /** e.g. `FLAC`, `AAC 320`, `MP3 128`. */
  label: string;
  lossless: boolean;
  /** Parson read this format from the station's audio (not its listing). */
  verified?: boolean;
}

export interface LiveStation {
  id: string;
  /** `radio:<id>`: play it like a song. */
  song_id: string;
  source: LiveStationSource;
  name: string;
  stream_url: string;
  homepage: string | null;
  /** Logo relayed by Parson (station hosts never see listeners). */
  logo_url: string | null;
  country: string | null;
  country_code: string | null;
  region: string | null;
  languages: string[];
  tags: string[];
  codec: string | null;
  bitrate_kbps: number | null;
  hls: boolean;
  quality: LiveQuality;
  votes: number | null;
  clicks: number | null;
  click_trend: number | null;
  /** The directory's last check reached the stream. */
  online: boolean | null;
  description: string | null;
  allow_lan: boolean;
  /** In the listener's My Live Radio. */
  saved: boolean;
  /** What Parson heard when it last opened the station, if it has. */
  verified_format?: LiveStreamFormat | null;
}

export type LiveContainer =
  "mp3" | "adts" | "flac" | "ogg" | "ts" | "fmp4" | "wav" | "unknown";

/** What a stream carries, read from its audio by the server. */
export interface LiveStreamFormat {
  container: LiveContainer;
  codec: string | null;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
  bitrate_kbps: number | null;
  /** The codec was read from the audio frames, not taken on trust. */
  verified: boolean;
}

export type LiveTransport = "http" | "hls";

export type LiveDelivery =
  | { mode: "original" }
  | { mode: "repackage"; output: string }
  | {
      mode: "convert";
      output: string;
      sample_rate: number | null;
      bit_depth: number | null;
      bitrate_kbps: number | null;
    };

export interface LiveTitle {
  raw: string;
  artist: string | null;
  title: string;
  /** Unix milliseconds. */
  at: number;
}

export interface LiveNowPlaying {
  station_id: string;
  /** Someone is listening through Parson right now. */
  live: boolean;
  title: LiveTitle | null;
  /** Earlier titles, newest first. */
  history: LiveTitle[];
  format: LiveStreamFormat | null;
  quality: LiveQuality | null;
  transport: LiveTransport | null;
  delivery: LiveDelivery | null;
  stream_name: string | null;
  genre: string | null;
}

export type LiveCategoryGroup = "collections" | "music" | "talk";

export type LiveOrder =
  "popular" | "trending" | "votes" | "bitrate" | "name" | "random";

export interface LiveCategory {
  id: string;
  label: string;
  group: LiveCategoryGroup;
  tag?: string;
  lossless?: boolean;
  min_bitrate?: number;
  order?: LiveOrder;
  /** Gradient colours for the tile. */
  colors: [string, string];
}

export interface LiveRadioHome {
  my: LiveStation[];
  recent: LiveStation[];
  popular: LiveStation[];
  lossless: LiveStation[];
  trending: LiveStation[];
  local: LiveStation[];
  categories: LiveCategory[];
  /** Set when the directory is unreachable; saved stations still play. */
  directory_error: string | null;
}

export interface LiveSearch {
  name?: string;
  tag?: string;
  /** ISO 3166-1 alpha-2. */
  country_code?: string;
  language?: string;
  codec?: string;
  min_bitrate?: number;
  lossless?: boolean;
  order?: LiveOrder;
  offset?: number;
  limit?: number;
}

export interface LiveStationPage {
  items: LiveStation[];
  offset: number;
  limit: number;
  has_more: boolean;
}

export interface LiveStationDetail {
  station: LiveStation;
  now_playing: LiveNowPlaying;
  similar: LiveStation[];
}

export type LiveFacetKind = "tags" | "countries" | "languages" | "codecs";

export interface LiveFacet {
  name: string;
  /** Country or language code. */
  code?: string;
  station_count: number;
}

/** What `checkLiveStream` found behind a URL. */
export interface LiveStreamCheck {
  resolved_url: string;
  transport: LiveTransport;
  format: LiveStreamFormat;
  quality: LiveQuality;
  stream_name: string | null;
  genre: string | null;
  title: string | null;
  titles_supported: boolean;
}

export interface NewLiveStation {
  url: string;
  name?: string;
  homepage?: string;
  logo_url?: string;
  tags?: string[];
  country_code?: string;
  language?: string;
  description?: string;
  /** LAN streams (private addresses); administrators only. */
  allow_lan?: boolean;
}

export interface LiveStationEdit {
  name?: string;
  url?: string;
  homepage?: string;
  logo_url?: string;
  tags?: string[];
  description?: string;
}

const LIVE_ID_PATTERN =
  /^radio:([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i;

/** The station id inside a `radio:<id>` song id; `null` otherwise. */
export function liveStationIdFromSongId(
  songId: string | null | undefined,
): string | null {
  return LIVE_ID_PATTERN.exec(songId ?? "")?.[1] ?? null;
}

/** Whether a song id is a live radio station. */
export const isLiveRadioId = (songId: string | null | undefined) =>
  liveStationIdFromSongId(songId) !== null;

export const liveRadioSongId = (stationId: string) => `radio:${stationId}`;

const path = (id: string) => encodeURIComponent(id);

/** Turns the server's relative logo path into an absolute URL. */
export function liveStationLogoUrl(
  station: Pick<LiveStation, "logo_url">,
  baseURL: string = getApiBaseURL(),
): string | null {
  if (!station.logo_url) return null;
  if (/^https?:\/\//.test(station.logo_url)) return station.logo_url;
  const origin = baseURL.replace(/\/api\/(?:music\/)?v1\/?$/, "");
  return `${origin}${station.logo_url}`;
}

export async function getLiveRadioHome(
  country?: string | null,
): Promise<LiveRadioHome> {
  const params = country ? { country } : undefined;
  return (await api.get<LiveRadioHome>("/live-radio/home", { params })).data;
}

export async function getLiveRadioCategories(): Promise<LiveCategory[]> {
  return (await api.get<{ items: LiveCategory[] }>("/live-radio/categories"))
    .data.items;
}

/** Drops empty values so the server sees only real filters. */
export function liveSearchParams(search: LiveSearch): Record<string, string> {
  const params: Record<string, string> = {};
  for (const [key, value] of Object.entries(search)) {
    if (value === undefined || value === null || value === "") continue;
    if (value === false) continue;
    params[key] = String(value).trim();
  }
  return params;
}

export async function searchLiveStations(
  search: LiveSearch,
): Promise<LiveStationPage> {
  return (
    await api.get<LiveStationPage>("/live-radio/stations", {
      params: liveSearchParams(search),
    })
  ).data;
}

export async function getLiveStation(id: string): Promise<LiveStationDetail> {
  return (await api.get<LiveStationDetail>(`/live-radio/stations/${path(id)}`))
    .data;
}

export async function getLiveFacets(
  kind: LiveFacetKind,
  limit = 200,
): Promise<LiveFacet[]> {
  return (
    await api.get<{ items: LiveFacet[] }>(`/live-radio/facets/${kind}`, {
      params: { limit },
    })
  ).data.items;
}

/** What several stations are playing (at most 50 ids). */
export async function getLiveNowPlaying(
  ids: string[],
): Promise<Record<string, LiveNowPlaying>> {
  const unique = [...new Set(ids)].slice(0, 50);
  if (!unique.length) return {};
  return (
    await api.get<{ items: Record<string, LiveNowPlaying> }>(
      "/live-radio/now-playing",
      { params: { ids: unique.join(",") }, timeout: 8_000 },
    )
  ).data.items;
}

/** Opens a URL on the server and reports its format and first title. */
export async function checkLiveStream(
  url: string,
  allowLan = false,
): Promise<LiveStreamCheck> {
  return (
    await api.post<LiveStreamCheck>(
      "/live-radio/check",
      { url, allow_lan: allowLan },
      { timeout: 30_000 },
    )
  ).data;
}

/**
 * Opens a station on the server and reads its real format from the audio
 * (a few seconds), so its quality badge shows what it actually sends.
 */
export async function verifyLiveStation(id: string): Promise<LiveStation> {
  return (
    await api.post<LiveStation>(
      `/live-radio/stations/${path(id)}/verify`,
      undefined,
      {
        timeout: 20_000,
      },
    )
  ).data;
}

export async function getMyLiveStations(): Promise<LiveStation[]> {
  return (await api.get<{ items: LiveStation[] }>("/live-radio/my")).data.items;
}

/** Saves a directory station to My Live Radio. */
export async function saveLiveStation(id: string): Promise<LiveStation> {
  return (await api.put<LiveStation>(`/live-radio/my/${path(id)}`)).data;
}

/** Adds a station by URL (checked by the server first). */
export async function addLiveStation(
  station: NewLiveStation,
): Promise<{ station: LiveStation; check: LiveStreamCheck }> {
  return (
    await api.post<{ station: LiveStation; check: LiveStreamCheck }>(
      "/live-radio/my",
      station,
      { timeout: 30_000 },
    )
  ).data;
}

export async function editLiveStation(
  id: string,
  edit: LiveStationEdit,
): Promise<LiveStation> {
  return (
    await api.patch<LiveStation>(`/live-radio/my/${path(id)}`, edit, {
      timeout: 30_000,
    })
  ).data;
}

export async function removeLiveStation(id: string): Promise<void> {
  await api.delete(`/live-radio/my/${path(id)}`);
}

export async function reorderLiveStations(ids: string[]): Promise<void> {
  await api.put("/live-radio/my-order", { ids });
}

export async function getRecentLiveStations(): Promise<LiveStation[]> {
  return (await api.get<{ items: LiveStation[] }>("/live-radio/recent")).data
    .items;
}

export async function clearRecentLiveStations(): Promise<void> {
  await api.delete("/live-radio/recent");
}
