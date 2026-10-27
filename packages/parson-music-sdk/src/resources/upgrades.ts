import api from "../core/http";
import type { FileQuality, ToolJob } from "./library-tools";

/**
 * Better versions: lossless copies requested from FETCH on the home network,
 * the fake-lossless check, and AI-enhanced copies made by your
 * enhancer worker. Admins request and enhance; everyone sees the status.
 *
 * AI enhancement generates plausible high frequencies; it never recovers
 * the original master. Enhanced copies are labelled "AI-enhanced", never
 * lossless or hi-res, and a real lossless copy always outranks them.
 */

const BASE = "/upgrades";
const ADMIN = "/upgrades/admin";

export type UpgradeRequestState =
  | "requested"
  | "downloading"
  | "handed_off"
  | "importing"
  | "upgraded"
  | "fake_lossless"
  | "no_match"
  | "not_found"
  | "failed"
  | "cancelled";

export interface UpgradeJob {
  job_id: string;
  /** FETCH's status, kept as FETCH wrote it (unknown values too). */
  status: string;
  /** 0 to 100. */
  progress: number | null;
  speed: string;
  eta: string;
  title: string;
  engine: string;
  dest: string;
  error: string;
  note: string;
  size: string;
}

export interface UpgradeRequest {
  id: number;
  album_id: string;
  album_title: string;
  artist_name: string;
  state: UpgradeRequestState;
  message: string;
  origin: "manual" | "automatic";
  reason: string;
  created_at: string;
  updated_at: string;
  progress: number | null;
  speed: string;
  eta: string;
  /** The app that owns a handed-off job (Lidarr and the like). */
  engine: string;
  jobs: UpgradeJob[];
  result_album_id: string | null;
  /** The text sent to FETCH (admins only). */
  query?: string;
}

export type UpgradeReasonKind =
  "lossy" | "low_bitrate" | "fake_lossless" | "ai_enhanced_only" | "cd_quality";

export interface UpgradeReason {
  kind: UpgradeReasonKind;
  label: string;
  detail: string;
  tracks: number;
}

export interface FetchSearchOutcome {
  /** `null` when FETCH's answer had no recognisable result list. */
  found: boolean | null;
  count: number;
  titles: string[];
}

export interface UpgradeCandidate {
  album_id: string;
  title: string;
  artist_id: string;
  artist_name: string;
  year: string;
  cover_url: string;
  musicbrainz_id: string;
  track_count: number;
  quality: FileQuality;
  reasons: UpgradeReason[];
  request: UpgradeRequest | null;
  enhanced_tracks: number;
  enhance_queued: number;
  search: FetchSearchOutcome | null;
}

export interface UpgradeCandidates {
  candidates: UpgradeCandidate[];
  requests: UpgradeRequest[];
  fetch_enabled: boolean;
  enhancer_enabled: boolean;
  auto_request: boolean;
  lossless_check: ToolJob | null;
}

export interface AlbumUpgradeStatus {
  album_id: string;
  request: UpgradeRequest | null;
  /** Tracks that play an AI-enhanced copy now. */
  enhanced_tracks: number;
  /** Tracks that have an AI-enhanced copy, playing or not. */
  enhanced_copies: number;
  enhance_queued: number;
}

export interface FetchSettings {
  enabled: boolean;
  base_url: string;
  /** The library folder FETCH saves into; `null` rescans everything. */
  library_root: string | null;
  auto_request: boolean;
  auto_per_day: number;
  include_cd_quality: boolean;
  move_replaced: boolean;
  replaced_folder: string | null;
}

export interface EnhancerSettings {
  enabled: boolean;
  base_url: string;
  output_folder: string | null;
  prefer_enhanced: boolean;
  shared_paths: boolean;
}

export interface UpgradeSettings {
  fetch: FetchSettings;
  enhancer: EnhancerSettings;
}

export type UpgradeSecret =
  | "fetch_token"
  | "cf_access_client_id"
  | "cf_access_client_secret"
  | "enhancer_token";

export interface FetchOpenApiReport {
  title: string | null;
  version: string | null;
  path_count: number;
  missing: string[];
}

export interface FetchCheck {
  ok: boolean;
  health: boolean;
  authorized: boolean;
  me: string | null;
  openapi: FetchOpenApiReport | null;
  free_space: string | null;
  error: string | null;
  checked_at: number;
}

export interface EnhancerInfo {
  worker_version: string;
  model: string;
  model_version: string;
  tag: string;
  device: string;
  device_name: string;
  sample_rate: number;
  realtime_factor: number | null;
  license: string;
}

export interface EnhancerCheck {
  ok: boolean;
  info: EnhancerInfo | null;
  realtime: boolean;
  error: string | null;
  checked_at: number;
}

export interface UpgradeSettingsView {
  settings: UpgradeSettings;
  /** Which secrets are saved; values never leave the server. */
  saved: UpgradeSecret[];
  library_roots: string[];
  enhanced_folder: string | null;
  fetch_check: FetchCheck | null;
  enhancer_check: EnhancerCheck | null;
}

export type EnhanceJobState =
  "queued" | "running" | "done" | "failed" | "cancelled" | "skipped";

export interface EnhanceJob {
  id: number;
  song_id: string;
  album_id: string;
  title: string;
  state: EnhanceJobState;
  progress: number;
  message: string;
  batch: string;
}

export interface EnhanceQueue {
  counts: { queued: number; running: number; done: number; failed: number };
  jobs: EnhanceJob[];
}

export interface RealtimeEnhanceStatus {
  available: boolean;
  model: string | null;
  realtime_factor: number | null;
  max_realtime_factor: number;
  status: {
    enhanced_chunks: number;
    original_chunks: number;
    gave_up: boolean;
  };
}

/* ---------------------------------------------------------------- */
/* Status for everyone                                               */
/* ---------------------------------------------------------------- */

export async function getAlbumUpgradeStatus(
  albumId: string,
): Promise<AlbumUpgradeStatus> {
  const response = await api.get<AlbumUpgradeStatus>(
    `${BASE}/albums/${encodeURIComponent(albumId)}`,
  );
  return response.data;
}

export async function getActiveUpgradeRequests(): Promise<UpgradeRequest[]> {
  const response = await api.get<{ requests: UpgradeRequest[] }>(
    `${BASE}/active`,
  );
  return Array.isArray(response.data?.requests) ? response.data.requests : [];
}

export async function getRealtimeEnhanceStatus(): Promise<RealtimeEnhanceStatus> {
  const response = await api.get<RealtimeEnhanceStatus>(`${BASE}/realtime`);
  return response.data;
}

/* ---------------------------------------------------------------- */
/* Admin                                                             */
/* ---------------------------------------------------------------- */

export async function getUpgradeSettings(): Promise<UpgradeSettingsView> {
  const response = await api.get<UpgradeSettingsView>(`${ADMIN}/settings`);
  return response.data;
}

export async function saveUpgradeSettings(
  settings: UpgradeSettings,
): Promise<UpgradeSettingsView> {
  const response = await api.put<UpgradeSettingsView>(
    `${ADMIN}/settings`,
    settings,
  );
  return response.data;
}

/** Saves a token on the server; it is never sent back to any client. */
export async function saveUpgradeSecret(
  name: UpgradeSecret,
  value: string,
): Promise<void> {
  await api.put(`${ADMIN}/secrets/${encodeURIComponent(name)}`, { value });
}

export async function clearUpgradeSecret(name: UpgradeSecret): Promise<void> {
  await api.delete(`${ADMIN}/secrets/${encodeURIComponent(name)}`);
}

export async function checkFetchConnection(): Promise<FetchCheck> {
  const response = await api.post<FetchCheck>(`${ADMIN}/fetch/check`);
  return response.data;
}

export async function checkEnhancerConnection(): Promise<EnhancerCheck> {
  const response = await api.post<EnhancerCheck>(`${ADMIN}/enhancer/check`);
  return response.data;
}

export async function getUpgradeCandidates(): Promise<UpgradeCandidates> {
  const response = await api.get<UpgradeCandidates>(`${ADMIN}/candidates`);
  return response.data;
}

export async function searchFetchForAlbum(
  albumId: string,
): Promise<FetchSearchOutcome> {
  const response = await api.post<FetchSearchOutcome>(
    `${ADMIN}/candidates/${encodeURIComponent(albumId)}/search`,
  );
  return response.data;
}

/** Asks FETCH for a lossless copy of each album. */
export async function requestLosslessVersions(
  albumIds: string[],
): Promise<number[]> {
  const response = await api.post<{ requested: number[] }>(
    `${ADMIN}/requests`,
    { album_ids: albumIds },
  );
  return Array.isArray(response.data?.requested) ? response.data.requested : [];
}

export async function cancelUpgradeRequest(id: number): Promise<void> {
  await api.post(`${ADMIN}/requests/${encodeURIComponent(String(id))}/cancel`);
}

export async function retryUpgradeRequest(id: number): Promise<void> {
  await api.post(`${ADMIN}/requests/${encodeURIComponent(String(id))}/retry`);
}

export async function deleteUpgradeRequest(id: number): Promise<void> {
  await api.delete(`${ADMIN}/requests/${encodeURIComponent(String(id))}`);
}

/** Runs the fake-lossless check (whole library, or some albums). */
export async function checkForFakeLossless(
  albumIds?: string[],
  force = false,
): Promise<ToolJob> {
  const response = await api.post<ToolJob>(`${ADMIN}/lossless-check`, {
    album_ids: albumIds,
    force,
  });
  return response.data;
}

export async function getEnhanceQueue(): Promise<EnhanceQueue> {
  const response = await api.get<EnhanceQueue>(`${ADMIN}/enhance`);
  return response.data;
}

export async function enhanceWithAi(target: {
  albumIds?: string[];
  songIds?: string[];
}): Promise<number> {
  const response = await api.post<{ queued: number }>(`${ADMIN}/enhance`, {
    album_ids: target.albumIds ?? [],
    song_ids: target.songIds ?? [],
  });
  return response.data?.queued ?? 0;
}

/** "Enhance all lossy that FETCH couldn't find". */
export async function enhanceUnfoundWithAi(): Promise<number> {
  const response = await api.post<{ queued: number }>(
    `${ADMIN}/enhance/unfound`,
  );
  return response.data?.queued ?? 0;
}

/** Plays an album's originals instead of its AI-enhanced copies (or back). */
export async function playOriginalsForAlbum(
  albumId: string,
  originals: boolean,
): Promise<number> {
  const response = await api.post<{ tracks: number }>(
    `${ADMIN}/albums/${encodeURIComponent(albumId)}/originals`,
    { originals },
  );
  return response.data?.tracks ?? 0;
}

export async function cancelEnhanceQueue(): Promise<number> {
  const response = await api.post<{ cancelled: number }>(
    `${ADMIN}/enhance/cancel`,
  );
  return response.data?.cancelled ?? 0;
}

/* ---------------------------------------------------------------- */
/* Labels                                                            */
/* ---------------------------------------------------------------- */

/** Still on its way (FETCH is working, or Mujin Music is importing). */
export function isUpgradeOpen(state: UpgradeRequestState): boolean {
  return (
    state === "requested" ||
    state === "downloading" ||
    state === "handed_off" ||
    state === "importing"
  );
}

function percent(value: number | null) {
  return value === null || !Number.isFinite(value)
    ? null
    : `${Math.round(Math.min(100, Math.max(0, value)))}%`;
}

/**
 * One line for the album page and the list, e.g. `Lossless version on its
 * way · 45%`, `Handed to Lidarr`, `Upgraded to lossless`.
 */
export function upgradeStatusLabel(request: UpgradeRequest): string {
  const progress = percent(request.progress);
  switch (request.state) {
    case "requested":
      return "Lossless version requested";
    case "downloading":
      return progress && request.progress
        ? `Lossless version on its way · ${progress}`
        : "Lossless version on its way";
    case "handed_off":
      return `Handed to ${request.engine.trim() || "another app"}`;
    case "importing":
      return "Lossless version arrived · checking it";
    case "upgraded":
      return "Upgraded to lossless";
    case "fake_lossless":
      return "FETCH's copy was fake lossless · original kept";
    case "no_match":
      return "FETCH's copy did not match · original kept";
    case "not_found":
      return "No lossless copy found";
    case "failed":
      return "Lossless request failed";
    case "cancelled":
      return "Lossless request cancelled";
    default:
      return String(request.state);
  }
}

/** Progress details for a running request: `3.2 MB/s · 1m 10s left`. */
export function upgradeProgressDetail(request: UpgradeRequest): string {
  return [request.speed, request.eta ? `${request.eta} left` : ""]
    .filter(Boolean)
    .join(" · ");
}

/** The FETCH query Mujin Music sends, mirrored for previews. */
export function fetchQueryPreview(album: {
  artist: string;
  title: string;
  year?: string;
  musicbrainzId?: string;
}): string {
  const clean = (value: string) =>
    value.replace(/["“”]/g, "'").replace(/\s+/g, " ").trim();
  const artist = clean(album.artist);
  let text = `the album ${clean(album.title)}`;
  if (artist && artist.toLowerCase() !== "unknown artist")
    text += ` by ${artist}`;
  const year = /^\d{4}/.exec(album.year ?? "")?.[0];
  const mbid =
    album.musicbrainzId &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
      album.musicbrainzId.trim(),
    )
      ? album.musicbrainzId.trim()
      : null;
  const details = [year, mbid ? `MusicBrainz release ${mbid}` : null].filter(
    Boolean,
  );
  if (details.length) text += ` (${details.join(", ")})`;
  return `${text} as flac`;
}

export const UPGRADE_REASON_LABELS: Record<UpgradeReasonKind, string> = {
  lossy: "Lossy",
  low_bitrate: "Low bitrate",
  fake_lossless: "Fake lossless",
  ai_enhanced_only: "AI-enhanced only",
  cd_quality: "CD quality",
};
