import api, { getApiBaseURL } from "../core/http";
import type { LyricsResult } from "./lyrics";
import type {
  ZoneKind,
  ZoneRepeatMode,
  ZoneStatus,
  ZoneStreamQuality,
} from "./zones";

/**
 * Displays are TVs, tablets, Chromecast tabs and spare browsers that show
 * what a zone plays, full screen and without controls (Roon's Displays).
 *
 * A screen pairs once, either by showing a six-digit code that a signed-in
 * app approves, or by opening a link the app makes. It then holds a key
 * that reads only the now-playing state of its zone (or of whichever zone
 * is playing): never the library, the account or any controls. The key
 * travels in the `X-Parson-Display` header; screen calls carry no session.
 */
export const DISPLAY_KEY_HEADER = "X-Parson-Display";
/** Where a paired screen keeps its key (per browser). */
export const DISPLAY_KEY_STORAGE = "mujin:display-key";

/** Timed lyrics over the artwork when a track has them, or never. */
export type DisplayLyricsMode = "auto" | "off";

export interface DisplayInfo {
  id: string;
  name: string;
  /** `null` follows whichever zone is playing. */
  zone_id: string | null;
  lyrics: DisplayLyricsMode;
  created_at: string;
  last_seen_at: string | null;
  /** Asked for its state within the last half minute. */
  online: boolean;
}

/** `hi_res`: lossless above 48 kHz or 16-bit; `lossless`: CD quality. */
export type DisplayFormatTier = "hi_res" | "lossless" | "lossy" | "unknown";

export interface DisplayTrackFormat {
  /** e.g. `FLAC 96 kHz/24-bit`, `MP3 320 kbps`. */
  label: string;
  tier: DisplayFormatTier;
  codec: string | null;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
}

export interface DisplayBiography {
  text: string;
  /** e.g. `English Wikipedia`. */
  source: string;
  url: string;
  license: string;
}

export interface DisplayTrack {
  song_id: string | null;
  title: string;
  artist: string;
  artist_id: string | null;
  album: string;
  album_id: string | null;
  year: string | null;
  composer: string | null;
  track_number: number | null;
  track_count: number | null;
  duration_ms: number;
  format: DisplayTrackFormat | null;
  /** Signed server path; resolve with {@link displayAssetURL}. */
  cover_url: string | null;
  artist_image_url: string | null;
  /** e.g. `Photo: Jane Doe, CC BY-SA 4.0`. */
  artist_image_credit: string | null;
  biography: DisplayBiography | null;
  /** A live radio stream: no seek bar or waveform. */
  live: boolean;
}

export interface DisplayZone {
  id: string;
  name: string;
  icon: string;
  kind: ZoneKind;
  status: ZoneStatus;
  position_ms: number;
  duration_ms: number;
  shuffle: boolean;
  repeat: ZoneRepeatMode;
  /** The server's rating of this zone's signal path, when it plans it. */
  quality: ZoneStreamQuality | null;
  bit_perfect: boolean | null;
  /** e.g. `FLAC 96 kHz/24-bit to Topping E30 (exclusive)`. */
  output: string | null;
  steps: string[];
}

export interface DisplayQueued {
  song_id: string;
  title: string;
  artist: string;
  cover_url: string | null;
}

export type DisplayStatus = "playing" | "nothing" | "zone_missing";

export interface DisplayState {
  display: DisplayInfo;
  server_time_ms: number;
  status: DisplayStatus;
  zone: DisplayZone | null;
  track: DisplayTrack | null;
  up_next: DisplayQueued[];
}

export interface DisplayPairing {
  pairing_id: string;
  /** Proves the poller is the screen that started the pairing. */
  secret: string;
  /** Six digits to show large. */
  code: string;
  /** Seconds. */
  expires_in: number;
}

export type DisplayPairingPoll =
  | { status: "pending"; name: string }
  | { status: "approved"; display: DisplayInfo; key: string }
  | { status: "expired" };

export interface DisplayWaveform {
  /** 0-255 per bin on a perceptual scale. */
  peaks: number[];
  rms: number[];
}

// ---------------------------------------------------------------------------
// Signed-in apps.

export async function getDisplays(): Promise<DisplayInfo[]> {
  const response = await api.get<{ displays: DisplayInfo[] }>("/displays", {
    timeout: 6_000,
  });
  return response.data.displays;
}

/** A display opened from a link; the key comes back once, for the link. */
export async function createDisplay(input: {
  name: string;
  zone_id?: string | null;
  lyrics?: DisplayLyricsMode;
}): Promise<{ display: DisplayInfo; key: string }> {
  const response = await api.post<{ display: DisplayInfo; key: string }>(
    "/displays",
    input,
    { timeout: 8_000 },
  );
  return response.data;
}

/** Approves the code a screen shows; the screen receives its key itself. */
export async function approveDisplay(input: {
  code: string;
  name?: string;
  zone_id?: string | null;
  lyrics?: DisplayLyricsMode;
}): Promise<DisplayInfo> {
  const response = await api.post<{ display: DisplayInfo }>(
    "/displays/approve",
    input,
    { timeout: 8_000 },
  );
  return response.data.display;
}

/** `zone_id: null` makes the display follow whichever zone plays. */
export async function updateDisplay(
  id: string,
  patch: {
    name?: string;
    zone_id?: string | null;
    lyrics?: DisplayLyricsMode;
  },
): Promise<DisplayInfo> {
  const response = await api.patch<{ display: DisplayInfo }>(
    `/displays/${encodeURIComponent(id)}`,
    patch,
    { timeout: 8_000 },
  );
  return response.data.display;
}

/** Removes a display; its key stops working at once. */
export async function removeDisplay(id: string): Promise<void> {
  await api.delete(`/displays/${encodeURIComponent(id)}`, { timeout: 8_000 });
}

// ---------------------------------------------------------------------------
// The screen.

function keyed(key: string, timeout: number) {
  return {
    anonymous: true,
    timeout,
    headers: { [DISPLAY_KEY_HEADER]: key },
  } as const;
}

export async function startDisplayPairing(
  name: string,
): Promise<DisplayPairing> {
  const response = await api.post<DisplayPairing>(
    "/display/pair",
    { name },
    { anonymous: true, timeout: 8_000 },
  );
  return response.data;
}

export async function pollDisplayPairing(
  pairing: Pick<DisplayPairing, "pairing_id" | "secret">,
): Promise<DisplayPairingPoll> {
  try {
    const response = await api.post<DisplayPairingPoll>(
      "/display/pair/status",
      { pairing_id: pairing.pairing_id, secret: pairing.secret },
      { anonymous: true, timeout: 8_000 },
    );
    return response.data;
  } catch (error) {
    if (displayErrorStatus(error) === 404) return { status: "expired" };
    throw error;
  }
}

export async function getDisplayState(key: string): Promise<DisplayState> {
  const response = await api.get<DisplayState>(
    "/display/state",
    keyed(key, 8_000),
  );
  return response.data;
}

export async function getDisplayLyrics(
  key: string,
  songId: string,
): Promise<LyricsResult> {
  const response = await api.get<LyricsResult>(
    `/display/lyrics/${encodeURIComponent(songId)}`,
    keyed(key, 30_000),
  );
  return response.data;
}

/**
 * The track's waveform, or `null` while the server works it out (ask again
 * in a few seconds).
 */
export async function getDisplayWaveform(
  key: string,
  songId: string,
): Promise<DisplayWaveform | null> {
  const response = await api.get<
    ({ status: "ready" } & DisplayWaveform) | { status: "analysing" }
  >(`/display/waveform/${encodeURIComponent(songId)}`, keyed(key, 8_000));
  return response.data.status === "ready"
    ? { peaks: response.data.peaks, rms: response.data.rms }
    : null;
}

/** The HTTP status of a failed display call, if it got that far. */
export function displayErrorStatus(error: unknown): number | null {
  const status = (error as { response?: { status?: number } } | null)?.response
    ?.status;
  return typeof status === "number" ? status : null;
}

/** The key is gone (display removed in Settings): pair again. */
export function isDisplayUnpaired(error: unknown): boolean {
  const status = displayErrorStatus(error);
  return status === 401 || status === 403;
}

// ---------------------------------------------------------------------------
// Pure helpers shared by the web and phone displays.

/** The server origin from the API base URL (`https://host:1993`). */
export function displayServerOrigin(apiBaseURL = getApiBaseURL()): string {
  return apiBaseURL.replace(/\/api\/(?:music\/)?v1\/?$/, "");
}

/** A server path from the display state as a full URL. */
export function displayAssetURL(
  path: string | null | undefined,
  origin = displayServerOrigin(),
): string | null {
  if (!path) return null;
  if (/^https?:\/\//.test(path)) return path;
  return `${origin}${path.startsWith("/") ? "" : "/"}${path}`;
}

/**
 * The link that opens a display on any browser. The key rides in the
 * fragment, so it never reaches server logs or a proxy.
 */
export function displayLink(origin: string, key: string): string {
  return `${origin.replace(/\/$/, "")}/display#key=${encodeURIComponent(key)}`;
}

/** The key from a display link's fragment (`#key=…`), if there is one. */
export function displayKeyFromHash(hash: string): string | null {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const key = params.get("key")?.trim();
  return key && key.startsWith("mjd_") && key.length <= 120 ? key : null;
}

/**
 * Where playback is now, from a state received at `receivedAtMs`: a
 * playing zone moves on with the clock, anything else holds still.
 */
export function estimateDisplayPosition(
  state: Pick<DisplayState, "zone" | "track">,
  receivedAtMs: number,
  nowMs: number,
): number {
  const zone = state.zone;
  if (!zone || !state.track) return 0;
  const duration = state.track.duration_ms || zone.duration_ms;
  const moving = zone.status === "playing";
  const position =
    zone.position_ms + (moving ? Math.max(0, nowMs - receivedAtMs) : 0);
  return duration > 0
    ? Math.min(Math.max(0, position), duration)
    : Math.max(0, position);
}

/**
 * How long to wait before asking again: quickly near a track's end (so the
 * next one shows on time), every two seconds while playing, more slowly
 * when nothing moves.
 */
export function displayPollDelay(
  state: Pick<DisplayState, "zone" | "track" | "status"> | null,
  positionMs: number,
): number {
  if (!state || state.status !== "playing" || !state.zone) return 4_000;
  if (state.zone.status !== "playing") return 3_000;
  const duration = state.track?.duration_ms ?? 0;
  if (duration > 0 && duration - positionMs < 4_000) return 700;
  return 2_000;
}

/** `3:07`, `1:02:45`. */
export function formatDisplayTime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = String(total % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${minutes}:${seconds}`;
}

export interface DisplayQualityChip {
  /** e.g. `FLAC 96 kHz/24-bit`. */
  format: string | null;
  /** `Hi-Res`, `CD quality`, `Lossy`. */
  tier: string | null;
  /**
   * The path rating when the server plans this zone's stream:
   * `Bit-perfect`, `Lossless`, `Enhanced`, `High quality`, `Low quality`.
   */
  path: string | null;
  /** The signal light's colour. */
  light: "lossless" | "enhanced" | "high" | "low" | "unknown";
  /**
   * The chip in a phone's width (`FLAC 24/96 · Hi-Res`): the same facts,
   * short enough to show whole instead of ending in "…".
   */
  compact: string;
}

/** `FLAC 24/96`, `ALAC 16/44.1`, `MP3`: the short format a phone shows. */
export function compactDisplayFormat(
  format: Pick<
    DisplayTrackFormat,
    "label" | "codec" | "sample_rate" | "bit_depth" | "tier"
  > | null,
): string | null {
  if (!format) return null;
  const codec = format.codec?.toUpperCase() ?? null;
  if (format.tier !== "lossy" && format.bit_depth && format.sample_rate) {
    const khz = format.sample_rate / 1000;
    const rate = Number.isInteger(khz) ? String(khz) : khz.toFixed(1);
    return `${codec ? `${codec} ` : ""}${format.bit_depth}/${rate}`;
  }
  return codec ?? format.label;
}

const TIER_TEXT: Record<DisplayFormatTier, string | null> = {
  hi_res: "Hi-Res",
  lossless: "CD quality",
  lossy: "Lossy",
  unknown: null,
};

const PATH_TEXT: Record<ZoneStreamQuality, string> = {
  lossless: "Lossless",
  enhanced: "Enhanced",
  high: "High quality",
  low: "Low quality",
};

/**
 * The quality chip: the file's format and tier, and the zone's signal path
 * rating when the server plans the stream (a browser zone decodes itself,
 * so only the file is known).
 */
export function displayQualityChip(
  track: Pick<DisplayTrack, "format" | "live"> | null,
  zone: Pick<DisplayZone, "quality" | "bit_perfect"> | null,
): DisplayQualityChip | null {
  if (!track) return null;
  const format = track.format?.label ?? null;
  const tier = track.format ? TIER_TEXT[track.format.tier] : null;
  const path = zone?.quality
    ? zone.bit_perfect
      ? "Bit-perfect"
      : PATH_TEXT[zone.quality]
    : null;
  let light: DisplayQualityChip["light"] = "unknown";
  if (zone?.quality) light = zone.quality;
  else if (track.format?.tier === "lossy") light = "low";
  else if (track.format?.tier === "hi_res" || track.format?.tier === "lossless")
    light = "lossless";
  if (!format && !tier && !path) return null;
  const compact = [compactDisplayFormat(track.format) ?? format, path ?? tier]
    .filter(Boolean)
    .join(" · ");
  return { format, tier, path, light, compact };
}

/**
 * `count` bars from a stored waveform, each the loudest peak of its span,
 * scaled 0..1 against the track's own loudest peak (a quiet recording keeps
 * its shape instead of lying flat). Empty when there is no waveform.
 */
export function waveformBars(
  waveform: Pick<DisplayWaveform, "peaks"> | null,
  count: number,
): number[] {
  const peaks = waveform?.peaks ?? [];
  if (!peaks.length || count <= 0) return [];
  const loudest = Math.max(32, ...peaks);
  const bars: number[] = [];
  for (let index = 0; index < count; index += 1) {
    const start = Math.floor((index * peaks.length) / count);
    const end = Math.max(
      start + 1,
      Math.floor(((index + 1) * peaks.length) / count),
    );
    let peak = 0;
    for (let at = start; at < end && at < peaks.length; at += 1) {
      peak = Math.max(peak, peaks[at] ?? 0);
    }
    bars.push(Math.min(1, Math.max(0, peak / loudest)));
  }
  return bars;
}

/**
 * A slow, small shift of the whole layout so a TV or OLED panel never
 * holds one picture for hours: a different spot every few minutes, never
 * more than `maxPx` from centre. Stands still when motion is reduced.
 */
export function displayDrift(
  nowMs: number,
  options: { maxPx?: number; periodMs?: number; reducedMotion?: boolean } = {},
): { x: number; y: number } {
  if (options.reducedMotion) return { x: 0, y: 0 };
  const maxPx = options.maxPx ?? 10;
  const period = options.periodMs ?? 4 * 60_000;
  const step = Math.floor(nowMs / period);
  // Points on a small circle, a quarter-turn plus a bit each step.
  const angle = step * 1.9;
  return {
    x: Math.round(Math.cos(angle) * maxPx),
    y: Math.round(Math.sin(angle) * maxPx * 0.6),
  };
}

/** The line under the clock: what the screen shows and from where. */
export function displayCaption(state: DisplayState | null): string {
  if (!state) return "Connecting…";
  switch (state.status) {
    case "zone_missing":
      return "The zone this display follows is switched off or private.";
    case "nothing":
      return state.display.zone_id
        ? "Nothing is playing in this zone."
        : "Nothing is playing. Start music in any zone.";
    default:
      return state.zone ? state.zone.name : "";
  }
}

/** `Track 3 of 12 · 2012`. */
export function displayTrackLine(track: DisplayTrack): string {
  const parts: string[] = [];
  // "of M" only when the library holds the whole release.
  if (
    track.track_number &&
    track.track_count &&
    track.track_number <= track.track_count
  )
    parts.push(`Track ${track.track_number} of ${track.track_count}`);
  else if (track.track_number) parts.push(`Track ${track.track_number}`);
  if (track.year) parts.push(track.year);
  return parts.join(" · ");
}
