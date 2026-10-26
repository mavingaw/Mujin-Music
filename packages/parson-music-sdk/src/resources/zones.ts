import api, { getApiWebSocketURL } from "../core/http";

/**
 * Zones are every place music can play: each running Parson app (web,
 * desktop, mobile), an active Chromecast session, each Sonos room or group,
 * each UPnP/DLNA renderer, each audio device (USB DAC, built-in, HDMI) of
 * the computer running the server, each device of a machine running Parson
 * Bridge (and sample-synced groups of them), each AirPlay receiver, each
 * Chromecast the server drives, and linked groups of zones. The server
 * keeps the list; clients register themselves, pick a zone to control,
 * transfer queues and group zones.
 */
export type ZoneKind =
  | "web"
  | "desktop"
  | "mobile"
  | "chromecast"
  | "sonos"
  | "upnp"
  | "device"
  | "bridge"
  | "airplay"
  | "linked";

/** How a server-driven zone changes volume. */
export type ZoneVolumeMode = "hardware" | "software" | "fixed";

/** A volume scale as the device reports it (a DAC mixer in dB, 0..30 steps). */
export interface DeviceVolumeRange {
  min: number;
  max: number;
  step: number;
  unit: "dB" | "steps" | "level";
  /** The device changes its own gain: samples stay untouched. */
  hardware: boolean;
  current?: number | null;
}

/** Per-zone DSP: preamp, bass/treble shelves and headphone crossfeed. */
export interface ZoneDsp {
  enabled: boolean;
  preamp_db: number;
  bass_db: number;
  treble_db: number;
  crossfeed: boolean;
}

/** Signal path of the playing track, from file to device. */
/**
 * Roon-style rating of a signal path, worst stage first: `low` when a lossy
 * file is involved, `high` when the output needs a conversion (sample rate,
 * bit depth, software volume), `enhanced` for processing the listener asked
 * for (leveling, DSP), `lossless` when the file reaches the output untouched.
 */
export type ZoneStreamQuality = "lossless" | "enhanced" | "high" | "low";

/** The playing track's path from file to output, as the server plans it. */
export interface ZoneStreamInfo {
  /** e.g. `FLAC 96 kHz/24-bit`. */
  source: string;
  /** e.g. `FLAC 48 kHz/24-bit to Living Room (Sonos Five)`. */
  output: string;
  bit_perfect: boolean;
  /** Processing in the order it runs. */
  steps: string[];
  /** Missing from servers older than the rating. */
  quality?: ZoneStreamQuality;
}

/** What a zone plays as-is and what happens to everything else. */
export interface ZoneOutputInfo {
  max_sample_rate: number;
  max_bit_depth: number;
  formats: string[];
  /** "Plays up to X kHz/Y-bit, converts the rest" in words. */
  note: string;
  exclusive?: boolean;
  volume_mode?: ZoneVolumeMode;
  stream?: ZoneStreamInfo;
}

export type ZoneStatus =
  "playing" | "paused" | "buffering" | "stopped" | "idle" | "unavailable";

export type ZoneSource = "none" | "parson" | "external";

/** `all` starts the queue over after the last track; `one` repeats a track. */
export type ZoneRepeatMode = "off" | "all" | "one";

export interface ZoneVolume {
  level: number;
  muted: boolean;
  /** Hard safety minimum (0 when unset). */
  min: number;
  /** Hard safety maximum (100 when unset). */
  max: number;
  /** Soft comfort limit: sliders stop here, +/- may go past it. */
  comfort?: number | null;
  /** The range the device itself reports (read-only). */
  device_min?: number;
  device_max?: number;
  step: number;
  /** The device's own scale, when it reports one. */
  device_range?: DeviceVolumeRange;
}

/** Roon's volume leveling: `auto` levels by album while an album plays in order. */
export type ZoneLevelingMode = "off" | "auto" | "track" | "album";

export interface ZoneSettingsView {
  /** The device's (or group's) own name. */
  default_name: string;
  custom_name: boolean;
  /** Custom icon, `null` for the device's icon. */
  icon: string | null;
  crossfade_seconds: number;
  leveling: ZoneLevelingMode;
  leveling_target_lufs: number;
  leveling_adjustment_db: number;
  /** Only the device the zone runs on sees and controls it. */
  private: boolean;
  dsp?: ZoneDsp;
  /** Audio devices: exclusive (bit-perfect) access. */
  exclusive?: boolean;
  volume_mode?: ZoneVolumeMode;
  max_sample_rate?: number | null;
  max_bit_depth?: number | null;
  supports: {
    crossfade: boolean;
    leveling: boolean;
    volume_limits: boolean;
    private: boolean;
    dsp?: boolean;
    exclusive?: boolean;
    volume_modes?: ZoneVolumeMode[];
    max_format?: boolean;
  };
  notes: string[];
}

/**
 * A settings change: only the fields present change. `name` never resets a
 * custom name; `reset_name: true` does. `null` clears icon and limits.
 */
export interface ZoneSettingsPatch {
  name?: string;
  reset_name?: boolean;
  icon?: string | null;
  crossfade_seconds?: number;
  leveling?: ZoneLevelingMode;
  leveling_target_lufs?: number;
  leveling_adjustment_db?: number;
  comfort_volume?: number | null;
  safety_min_volume?: number | null;
  safety_max_volume?: number | null;
  private?: boolean;
  dsp?: ZoneDsp;
  exclusive?: boolean;
  volume_mode?: ZoneVolumeMode;
  max_sample_rate?: number | null;
  max_bit_depth?: number | null;
}

export interface ZoneGroupMember {
  id: string;
  name: string;
  kind: ZoneKind;
  volume: ZoneVolume | null;
}

export interface ZoneGroupInfo {
  id: string;
  /** True only for native groups (Sonos, Parson Bridge) that play sample-synced. */
  synced: boolean;
  sync_note: string;
  members: ZoneGroupMember[];
}

export interface ZoneCapabilities {
  transport: boolean;
  seek: boolean;
  volume: boolean;
  mute: boolean;
  /** Lists the queue. */
  queue: boolean;
  /** Reorders, removes and clears upcoming items. */
  queue_edit: boolean;
  repeat: boolean;
  shuffle: boolean;
  group: boolean;
  transfer_target: boolean;
  sleep_timer: boolean;
}

export interface ZoneNowPlaying {
  song_id: string | null;
  title: string;
  artist: string;
  album: string;
  album_id: string | null;
  cover_url: string | null;
  duration_ms: number;
}

export interface Zone {
  id: string;
  kind: ZoneKind;
  ecosystem:
    | "parson"
    | "chromecast"
    | "sonos"
    | "upnp"
    | "device"
    | "bridge"
    | "airplay"
    | "linked";
  name: string;
  /** Icon name from `ZONE_ICONS` (custom or the device's). */
  icon?: string;
  device: string;
  status: ZoneStatus;
  source: ZoneSource;
  now_playing: ZoneNowPlaying | null;
  queue: { song_ids: string[]; current_position: number };
  repeat: ZoneRepeatMode;
  shuffle: boolean;
  position_ms: number;
  duration_ms: number;
  volume: ZoneVolume | null;
  group: ZoneGroupInfo | null;
  grouped_into: string | null;
  capabilities: ZoneCapabilities;
  sleep_timer: {
    ends_at_ms: number;
    /** Letting a nearly finished track end before pausing. */
    finishing_track: boolean;
    /** Fading out before pausing. */
    fading: boolean;
  } | null;
  settings?: ZoneSettingsView;
  /** Format limits and the signal path (Sonos, Chromecast, UPnP, devices). */
  output?: ZoneOutputInfo | null;
  notes: string[];
}

/** A Parson Bridge: another machine whose audio devices are zones. */
export interface BridgeStatus {
  address: string;
  source: "discovered" | "manual";
  connected: boolean;
  bridge_id: string | null;
  name: string | null;
  host: string | null;
  /** e.g. `linux aarch64`. */
  platform: string | null;
  version: string | null;
  /** `ALSA`, `Simulated`, or null. */
  backend: string | null;
  devices: number;
  /** Bridge clock minus server clock, in microseconds. */
  clock_offset_us: number | null;
  clock_uncertainty_us: number | null;
  /** Why the bridge lists no devices (no supported audio API). */
  note: string | null;
  last_error: string | null;
}

export interface AirplayReceiver {
  id: string;
  name: string;
  model: string | null;
  address: string;
  supported: boolean;
  reason: string | null;
}

export interface CastDevice {
  id: string;
  name: string;
  model: string;
  address: string;
  /** A Google Home speaker group. */
  group: boolean;
}

export interface ZoneOutputsStatus {
  upnp: { enabled: boolean; renderers: number; last_error: string | null };
  bridges?: { enabled: boolean; bridges: BridgeStatus[] };
  airplay?: {
    enabled: boolean;
    receivers: AirplayReceiver[];
    note: string | null;
  };
  chromecast?: { enabled: boolean; devices: CastDevice[]; note: string | null };
  devices: {
    enabled: boolean;
    /** `ALSA`, `Simulated`, or `null` when unavailable. */
    backend: string | null;
    devices: number;
    note: string | null;
  };
}

export interface ZoneList {
  zones: Zone[];
  server_time_ms: number;
  sonos: { enabled: boolean; players: number; last_error: string | null };
  outputs?: ZoneOutputsStatus;
}

export type ZoneCommand =
  | { command: "play" }
  | { command: "pause" }
  | { command: "play_pause" }
  | { command: "stop" }
  | { command: "next" }
  | { command: "previous" }
  | { command: "seek"; position_ms: number }
  | { command: "jump"; queue_position: number }
  | { command: "set_volume"; volume: number; member_id?: string }
  | { command: "adjust_volume"; delta: number; member_id?: string }
  | { command: "set_mute"; muted: boolean; member_id?: string }
  | {
      command: "load";
      song_ids: string[];
      current_position?: number;
      position_ms?: number;
      play?: boolean;
    }
  /** `song_id` guards against editing a queue that changed meanwhile. */
  | { command: "move_queue_item"; from: number; to: number; song_id?: string }
  | { command: "remove_queue_item"; queue_position: number; song_id?: string }
  | { command: "clear_upcoming" }
  | { command: "set_repeat"; mode: ZoneRepeatMode }
  | { command: "set_shuffle"; enabled: boolean };

export interface ClientZoneReport {
  kind: "web" | "desktop" | "mobile";
  name: string;
  device?: string;
  status: ZoneStatus;
  /** Send only when the queue changed (or when the server asks). */
  song_ids?: string[];
  current_position: number;
  position_ms: number;
  duration_ms: number;
  volume: number;
  muted: boolean;
  acknowledged_seq: number;
  repeat?: ZoneRepeatMode;
  shuffle?: boolean;
}

export interface PendingZoneCommand {
  seq: number;
  command: ZoneCommand;
}

export interface ClientZoneRegistration {
  zone_id: string;
  commands: PendingZoneCommand[];
  needs_queue: boolean;
}

/** Events on the zone WebSocket. */
export type ZoneSocketMessage =
  | { type: "ready" }
  | { type: "zones_changed" }
  | {
      type: "zone_command";
      zone_id: string;
      seq: number;
      command: ZoneCommand;
    };

export const MAX_ZONE_QUEUE_ITEMS = 500;

const zonePath = (zoneId: string) => `/zones/${encodeURIComponent(zoneId)}`;

/** Header naming this app's own zone device (for private zones). */
export const ZONE_DEVICE_HEADER = "X-Parson-Zone-Device";
let zoneViewerDevice: string | null = null;

/**
 * Names this app's zone device on zone requests, so the server shows this
 * device its private zone and hides other devices' private zones.
 */
export function setZoneViewerDevice(deviceId: string | null): void {
  zoneViewerDevice = deviceId || null;
}

/** Request options with the zone device header when one is set. */
function zoneRequest<T extends object>(options: T): T {
  return zoneViewerDevice
    ? { ...options, headers: { [ZONE_DEVICE_HEADER]: zoneViewerDevice } }
    : options;
}

export async function getZones(): Promise<ZoneList> {
  const response = await api.get<ZoneList>(
    "/zones",
    zoneRequest({ timeout: 5_000 }),
  );
  return response.data;
}

export function getZoneEventsURL(): string {
  return getApiWebSocketURL("/zones/events");
}

export async function reportClientZone(
  deviceId: string,
  report: ClientZoneReport,
): Promise<ClientZoneRegistration> {
  const response = await api.put<ClientZoneRegistration>(
    `/zones/clients/${encodeURIComponent(deviceId)}`,
    {
      ...report,
      song_ids: report.song_ids?.slice(0, MAX_ZONE_QUEUE_ITEMS),
    },
    { timeout: 5_000 },
  );
  return response.data;
}

export async function removeClientZone(deviceId: string): Promise<void> {
  await api.delete(`/zones/clients/${encodeURIComponent(deviceId)}`, {
    timeout: 3_000,
  });
}

export async function sendZoneCommand(
  zoneId: string,
  command: ZoneCommand,
): Promise<void> {
  await api.post(
    `${zonePath(zoneId)}/commands`,
    command,
    zoneRequest({ timeout: 8_000 }),
  );
}

/** Moves the queue, track and position from one zone to another. */
export async function transferZone(
  from: string,
  to: string,
): Promise<{ zone: Zone | null }> {
  const response = await api.post<{ zone: Zone | null }>(
    "/zones/transfer",
    { from, to },
    zoneRequest({ timeout: 30_000 }),
  );
  return response.data;
}

export async function groupZones(zoneIds: string[]): Promise<{
  group_id: string;
  synced: boolean;
  sync_note: string;
}> {
  const response = await api.post(
    "/zones/groups",
    { zone_ids: zoneIds },
    zoneRequest({ timeout: 15_000 }),
  );
  return response.data;
}

export async function ungroupZones(
  groupId: string,
  memberId?: string,
): Promise<void> {
  await api.delete(`/zones/groups/${encodeURIComponent(groupId)}`, {
    params: memberId ? { member: memberId } : undefined,
    timeout: 15_000,
  });
}

export async function updateZoneSettings(
  zoneId: string,
  patch: ZoneSettingsPatch,
): Promise<{ settings: unknown; zone: Zone | null }> {
  const response = await api.patch(
    `${zonePath(zoneId)}/settings`,
    patch,
    zoneRequest({}),
  );
  return response.data;
}

export interface Loudness {
  integrated_lufs: number;
  true_peak_db: number;
}

export interface TrackLoudness {
  track: Loudness | null;
  /** Present once every track of the album is measured. */
  album: Loudness | null;
}

/**
 * EBU R128 loudness for volume leveling (at most 20 songs). Songs not
 * measured yet come back `null` with `pending: true`; ask again shortly.
 */
export async function getZoneLoudness(songIds: string[]): Promise<{
  tracks: Record<string, TrackLoudness | null>;
  pending: boolean;
}> {
  const response = await api.post(
    "/zones/loudness",
    { song_ids: songIds.slice(0, 20) },
    { timeout: 30_000 },
  );
  return response.data;
}

/** Pauses the zone after `minutes`; `null` cancels the timer. */
export async function setZoneSleepTimer(
  zoneId: string,
  minutes: number | null,
): Promise<void> {
  await api.post(`${zonePath(zoneId)}/sleep`, { minutes }, zoneRequest({}));
}

/** Pauses every playing zone this app can see (Roon's "Pause all"). */
export async function pauseAllZones(): Promise<{ paused: number }> {
  const response = await api.post<{ paused: number }>(
    "/zones/pause-all",
    {},
    zoneRequest({ timeout: 15_000 }),
  );
  return response.data;
}

/** Looks for UPnP renderers and audio devices now. */
export async function discoverOutputs(): Promise<ZoneOutputsStatus> {
  const response = await api.post<ZoneOutputsStatus>(
    "/zones/outputs/discover",
    {},
    { timeout: 20_000 },
  );
  return response.data;
}

export async function getBridges(): Promise<{
  enabled: boolean;
  bridges: BridgeStatus[];
}> {
  const response = await api.get("/zones/bridges", zoneRequest({}));
  return response.data;
}

/** Adds a Parson Bridge by address (`host` or `host:port`); admins only. */
export async function addBridge(address: string): Promise<BridgeStatus> {
  const response = await api.post<BridgeStatus>(
    "/zones/bridges",
    { address },
    { timeout: 20_000 },
  );
  return response.data;
}

export async function removeBridge(address: string): Promise<void> {
  await api.delete(`/zones/bridges/${encodeURIComponent(address)}`);
}

export async function discoverSonos(): Promise<{
  players: number;
  last_error?: string;
}> {
  const response = await api.post(
    "/zones/sonos/discover",
    {},
    { timeout: 20_000 },
  );
  return response.data;
}

// ----- pure helpers shared by web and mobile -------------------------------

const KIND_ORDER: Record<ZoneKind, number> = {
  web: 1,
  desktop: 1,
  mobile: 1,
  device: 2,
  bridge: 2,
  linked: 3,
  sonos: 4,
  airplay: 5,
  upnp: 5,
  chromecast: 6,
};

/**
 * Zones ordered for a picker: this device first, then apps, this
 * computer's audio devices, groups, Sonos, UPnP renderers, Cast.
 */
export function orderZones(zones: Zone[], localZoneId?: string | null): Zone[] {
  return [...zones].sort((left, right) => {
    if (left.id === localZoneId) return -1;
    if (right.id === localZoneId) return 1;
    const kind = KIND_ORDER[left.kind] - KIND_ORDER[right.kind];
    return kind || left.name.localeCompare(right.name);
  });
}

export function zoneKindLabel(kind: ZoneKind): string {
  switch (kind) {
    case "web":
      return "Web browser";
    case "desktop":
      return "Desktop app";
    case "mobile":
      return "Phone or tablet";
    case "chromecast":
      return "Chromecast";
    case "sonos":
      return "Sonos";
    case "upnp":
      return "UPnP/DLNA renderer";
    case "device":
      return "Audio device";
    case "bridge":
      return "Parson Bridge";
    case "airplay":
      return "AirPlay";
    case "linked":
      return "Linked zones";
  }
}

/**
 * One line about the network outputs the server plays to, e.g.
 * `2 Parson Bridges (5 outputs) · 1 AirPlay receiver · 1 Chromecast`.
 */
export function zoneOutputsSummary(
  outputs: ZoneOutputsStatus | null | undefined,
): string | null {
  const parts: string[] = [];
  const bridges = (outputs?.bridges?.bridges ?? []).filter(
    (bridge) => bridge.connected,
  );
  if (bridges.length) {
    const devices = bridges.reduce((sum, bridge) => sum + bridge.devices, 0);
    parts.push(
      `${bridges.length} Parson ${bridges.length === 1 ? "Bridge" : "Bridges"} (${devices} ${devices === 1 ? "output" : "outputs"})`,
    );
  }
  const airplay = (outputs?.airplay?.receivers ?? []).filter(
    (receiver) => receiver.supported,
  ).length;
  if (airplay) {
    parts.push(
      `${airplay} AirPlay ${airplay === 1 ? "receiver" : "receivers"}`,
    );
  }
  const cast = outputs?.chromecast?.devices.length ?? 0;
  if (cast) parts.push(`${cast} ${cast === 1 ? "Chromecast" : "Chromecasts"}`);
  return parts.length ? parts.join(" · ") : null;
}

/** Whether pressing "Pause all" would pause anything. */
export function anyZonePlaying(zones: Zone[]): boolean {
  return zones.some(
    (zone) =>
      !zone.grouped_into &&
      zone.capabilities.transport &&
      (zone.status === "playing" || zone.status === "buffering"),
  );
}

/** `96 kHz`, `44.1 kHz`. */
export function formatSampleRate(rate: number): string {
  if (rate % 1000 === 0) return `${rate / 1000} kHz`;
  return `${(Math.round(rate / 100) / 10).toFixed(1)} kHz`;
}

const signed = (value: number) =>
  value > 0 ? `+${value}` : value < 0 ? `\u2212${Math.abs(value)}` : "0";

/** Headroom management: the level drops by the largest boost. */
export function dspEffectivePreamp(dsp: ZoneDsp): number {
  const boost = Math.max(dsp.bass_db, dsp.treble_db, 0);
  return Math.min(dsp.preamp_db, -boost);
}

export function isDspActive(dsp: ZoneDsp | null | undefined): boolean {
  return Boolean(
    dsp?.enabled &&
    (dsp.preamp_db !== 0 ||
      dsp.bass_db !== 0 ||
      dsp.treble_db !== 0 ||
      dsp.crossfeed),
  );
}

/** Short DSP description, matching the server's signal path wording. */
export function dspSummary(dsp: ZoneDsp | null | undefined): string {
  if (!dsp || !isDspActive(dsp)) return "Off";
  const parts: string[] = [];
  const preamp = dspEffectivePreamp(dsp);
  if (preamp !== 0) parts.push(`Preamp ${signed(preamp)} dB`);
  if (dsp.bass_db !== 0) parts.push(`Bass ${signed(dsp.bass_db)} dB`);
  if (dsp.treble_db !== 0) parts.push(`Treble ${signed(dsp.treble_db)} dB`);
  if (dsp.crossfeed) parts.push("Crossfeed");
  return parts.join(", ");
}

export const DEFAULT_ZONE_DSP: ZoneDsp = {
  enabled: false,
  preamp_db: 0,
  bass_db: 0,
  treble_db: 0,
  crossfeed: false,
};

export function volumeModeLabel(mode: ZoneVolumeMode): string {
  switch (mode) {
    case "hardware":
      return "Device volume (bit-perfect)";
    case "software":
      return "Parson volume (not bit-perfect)";
    case "fixed":
      return "Fixed volume (set it on the amplifier)";
  }
}

/** The device's own reading, e.g. `-16.0 dB` or `15 of 30`. */
export function deviceVolumeLabel(volume: ZoneVolume | null): string | null {
  const range = volume?.device_range;
  if (!range) return null;
  const current = range.current;
  if (range.unit === "dB") {
    const span = `${range.min.toFixed(1)} to ${range.max.toFixed(1)} dB`;
    return current === null || current === undefined
      ? `Device range ${span}`
      : `${current.toFixed(1)} dB on the device (${span})`;
  }
  if (range.unit === "level") {
    return current === null || current === undefined
      ? "The device's own volume"
      : `${Math.round(current * 100)}% on the device`;
  }
  return current === null || current === undefined
    ? `Device range ${range.min}-${range.max}`
    : `${current} of ${range.max} on the device`;
}

const STREAM_QUALITY_TEXT: Record<
  ZoneStreamQuality,
  { label: string; description: string }
> = {
  lossless: {
    label: "Lossless",
    description: "The file reaches the output without being changed.",
  },
  enhanced: {
    label: "Enhanced",
    description:
      "Parson processes the audio because you asked it to (leveling or DSP).",
  },
  high: {
    label: "High quality",
    description:
      "Converted so the output can play it (sample rate, bit depth or volume), without lossy compression.",
  },
  low: {
    label: "Low quality",
    description: "A lossy file or lossy stream is part of the path.",
  },
};

/** The signal path rating of `stream`, inferring it for older servers. */
export function streamQuality(stream: ZoneStreamInfo): ZoneStreamQuality {
  if (stream.quality) return stream.quality;
  return stream.bit_perfect ? "lossless" : "high";
}

/** Label and one-sentence meaning of a signal path rating. */
export function streamQualityText(quality: ZoneStreamQuality): {
  label: string;
  description: string;
} {
  return STREAM_QUALITY_TEXT[quality];
}

/**
 * A one-line quality badge: `Bit-perfect`, `Enhanced`, `High quality`,
 * `Low quality`, or `null` when nothing plays.
 */
export function zoneQualityLabel(zone: Zone): string | null {
  const stream = zone.output?.stream;
  if (!stream) return null;
  if (stream.bit_perfect) return "Bit-perfect";
  return STREAM_QUALITY_TEXT[streamQuality(stream)].label;
}

export function zoneStatusLabel(zone: Zone): string {
  if (zone.source === "external") return "Playing from another app";
  switch (zone.status) {
    case "playing":
      return "Playing";
    case "paused":
      return "Paused";
    case "buffering":
      return "Loading";
    case "stopped":
      return "Stopped";
    case "unavailable":
      return "Unavailable";
    default:
      return "Ready";
  }
}

/**
 * What grouping these zones produces. Sonos with Sonos groups natively and
 * plays sample-synced; any other mix is a linked group that is not
 * sample-synced. Chromecast groups are made in Google Home.
 */
export function groupingPlan(zones: Zone[]): {
  allowed: boolean;
  synced: boolean;
  reason: string;
} {
  if (zones.length < 2) {
    return {
      allowed: false,
      synced: false,
      reason: "Choose at least two zones.",
    };
  }
  if (zones.some((zone) => zone.kind === "chromecast")) {
    return {
      allowed: false,
      synced: false,
      reason: "Group Chromecast speakers in the Google Home app.",
    };
  }
  if (zones.some((zone) => zone.kind === "linked")) {
    return {
      allowed: false,
      synced: false,
      reason: "Ungroup the linked zone first.",
    };
  }
  if (zones.every((zone) => zone.kind === "sonos")) {
    return {
      allowed: true,
      synced: true,
      reason: "Native Sonos group: rooms play sample-synced.",
    };
  }
  if (zones.every((zone) => zone.kind === "bridge")) {
    if (zones.some((zone) => zone.group)) {
      return {
        allowed: false,
        synced: false,
        reason: "Ungroup the bridge group first.",
      };
    }
    return {
      allowed: true,
      synced: true,
      reason: `Parson Bridge group: sample-synced to ${zones[0]?.name ?? "the first zone"}'s DAC clock, which stays bit-perfect.`,
    };
  }
  return {
    allowed: true,
    synced: false,
    reason:
      "Linked group: zones start together and share controls, but are not sample-synced across ecosystems.",
  };
}

/** Estimated playback position now, from a list fetched at `fetchedAtMs`. */
export function estimateZonePosition(
  zone: Pick<Zone, "position_ms" | "duration_ms" | "status">,
  fetchedAtMs: number,
  nowMs: number,
): number {
  const elapsed =
    zone.status === "playing" ? Math.max(0, nowMs - fetchedAtMs) : 0;
  const position = zone.position_ms + elapsed;
  return zone.duration_ms > 0
    ? Math.min(position, zone.duration_ms)
    : Math.max(0, position);
}

/** Commands applied in order exactly once, even when they arrive twice. */
export function unseenZoneCommands(
  commands: PendingZoneCommand[],
  lastAppliedSeq: number,
): PendingZoneCommand[] {
  const seen = new Set<number>();
  return [...commands]
    .sort((left, right) => left.seq - right.seq)
    .filter((command) => {
      if (command.seq <= lastAppliedSeq || seen.has(command.seq)) return false;
      seen.add(command.seq);
      return true;
    });
}

/** A stable, random id for this installation's client zone. */
export function createZoneDeviceId(random: () => number = Math.random): string {
  const alphabet = "abcdefghijklmnopqrstuvwxyz0123456789";
  let id = "";
  for (let index = 0; index < 20; index += 1) {
    id += alphabet[Math.floor(random() * alphabet.length) % alphabet.length];
  }
  return id;
}

export function formatSleepRemaining(endsAtMs: number, nowMs: number): string {
  const minutes = Math.max(0, Math.ceil((endsAtMs - nowMs) / 60_000));
  if (minutes >= 60) {
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    return rest ? `${hours} h ${rest} min` : `${hours} h`;
  }
  return `${minutes} min`;
}

/** Whether queue item `index` can be moved or removed (upcoming only). */
export function isEditableQueueItem(
  zone: Pick<Zone, "queue" | "capabilities">,
  index: number,
): boolean {
  return (
    zone.capabilities.queue_edit &&
    index > zone.queue.current_position &&
    index < zone.queue.song_ids.length
  );
}

/**
 * Applies a queue edit to a copy of `songIds` the way the server does, for
 * optimistic display. Returns `null` when the edit is not allowed: only
 * upcoming items (after `current`) move or are removed.
 */
export function previewQueueEdit(
  songIds: readonly string[],
  current: number,
  command: ZoneCommand,
): string[] | null {
  const next = [...songIds];
  switch (command.command) {
    case "move_queue_item": {
      const { from, to } = command;
      if (
        from === to ||
        from <= current ||
        to <= current ||
        from >= next.length ||
        to >= next.length ||
        (command.song_id !== undefined && next[from] !== command.song_id)
      ) {
        return null;
      }
      const [item] = next.splice(from, 1);
      next.splice(to, 0, item!);
      return next;
    }
    case "remove_queue_item": {
      const position = command.queue_position;
      if (
        position <= current ||
        position >= next.length ||
        (command.song_id !== undefined && next[position] !== command.song_id)
      ) {
        return null;
      }
      next.splice(position, 1);
      return next;
    }
    case "clear_upcoming":
      return next.slice(0, current + 1);
    default:
      return next;
  }
}

/** The next repeat mode when cycling a single repeat button. */
export function nextRepeatMode(mode: ZoneRepeatMode): ZoneRepeatMode {
  return mode === "off" ? "all" : mode === "all" ? "one" : "off";
}

export function repeatModeLabel(mode: ZoneRepeatMode): string {
  return mode === "all"
    ? "Repeat queue"
    : mode === "one"
      ? "Repeat track"
      : "Repeat off";
}

/** Short sleep timer status for zone lists. */
export function sleepTimerLabel(
  timer: NonNullable<Zone["sleep_timer"]>,
  nowMs: number,
): string {
  if (timer.fading) return "Fading out to sleep";
  if (timer.finishing_track) return "Sleep after this track";
  return `Sleep in ${formatSleepRemaining(timer.ends_at_ms, nowMs)}`;
}

// ----- zone settings --------------------------------------------------------

/** Icons a zone can wear (the server accepts exactly these). */
export const ZONE_ICONS = [
  "speaker",
  "headphones",
  "monitor",
  "laptop",
  "phone",
  "tablet",
  "tv",
  "radio",
  "turntable",
  "amplifier",
  "car",
  "bed",
  "sofa",
  "kitchen",
  "office",
  "garden",
  "music",
] as const;
export type ZoneIconName = (typeof ZONE_ICONS)[number];

const DEFAULT_ICONS: Record<ZoneKind, ZoneIconName> = {
  web: "monitor",
  desktop: "laptop",
  mobile: "phone",
  chromecast: "tv",
  sonos: "speaker",
  upnp: "radio",
  device: "amplifier",
  bridge: "amplifier",
  airplay: "speaker",
  linked: "music",
};

/** The icon a zone shows: its custom icon, else its kind's. */
export function zoneIconName(zone: Pick<Zone, "icon" | "kind">): ZoneIconName {
  const icon = zone.icon as ZoneIconName | undefined;
  return icon && ZONE_ICONS.includes(icon) ? icon : DEFAULT_ICONS[zone.kind];
}

export const CROSSFADE_CHOICES = [0, 1, 2, 3, 4, 5, 6, 8, 10, 12] as const;
export const DEFAULT_LEVELING_TARGET_LUFS = -18;
export const LEVELING_TARGET_RANGE = { min: -30, max: -5 } as const;
export const LEVELING_ADJUSTMENT_RANGE = { min: -12, max: 12 } as const;

export function levelingModeLabel(mode: ZoneLevelingMode): string {
  switch (mode) {
    case "auto":
      return "Auto";
    case "track":
      return "Track";
    case "album":
      return "Album";
    default:
      return "Off";
  }
}

/** What the settings form edits. Volumes are 0..100; `null` is "no limit". */
export interface ZoneSettingsDraft {
  name: string;
  icon: string | null;
  crossfade_seconds: number;
  leveling: ZoneLevelingMode;
  leveling_target_lufs: number;
  leveling_adjustment_db: number;
  comfort_volume: number | null;
  safety_min_volume: number | null;
  safety_max_volume: number | null;
  private: boolean;
  dsp: ZoneDsp;
  exclusive: boolean;
  volume_mode: ZoneVolumeMode;
  max_sample_rate: number | null;
  max_bit_depth: number | null;
}

/** The form's starting values for `zone`. */
export function zoneSettingsDraft(zone: Zone): ZoneSettingsDraft {
  const settings = zone.settings;
  const volume = zone.volume;
  return {
    name: zone.name,
    icon: settings?.icon ?? null,
    crossfade_seconds: settings?.crossfade_seconds ?? 0,
    leveling: settings?.leveling ?? "off",
    leveling_target_lufs:
      settings?.leveling_target_lufs ?? DEFAULT_LEVELING_TARGET_LUFS,
    leveling_adjustment_db: settings?.leveling_adjustment_db ?? 0,
    comfort_volume: volume?.comfort ?? null,
    safety_min_volume: volume && volume.min > 0 ? volume.min : null,
    safety_max_volume: volume && volume.max < 100 ? volume.max : null,
    private: settings?.private ?? false,
    dsp: { ...DEFAULT_ZONE_DSP, ...settings?.dsp },
    exclusive: settings?.exclusive ?? true,
    volume_mode: settings?.volume_mode ?? "hardware",
    max_sample_rate: settings?.max_sample_rate ?? null,
    max_bit_depth: settings?.max_bit_depth ?? null,
  };
}

export const SAMPLE_RATE_CHOICES = [
  44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
] as const;
export const BIT_DEPTH_CHOICES = [16, 24, 32] as const;

/**
 * The request for the changes between `initial` and `draft`: unchanged
 * fields are left out, so saving never touches a name the user did not edit.
 * Clearing the name resets it to the device's name.
 */
export function zoneSettingsPatch(
  initial: ZoneSettingsDraft,
  draft: ZoneSettingsDraft,
): ZoneSettingsPatch {
  const patch: ZoneSettingsPatch = {};
  const name = draft.name.trim();
  if (name !== initial.name.trim()) {
    if (name) patch.name = name;
    else patch.reset_name = true;
  }
  const keys = [
    "icon",
    "crossfade_seconds",
    "leveling",
    "leveling_target_lufs",
    "leveling_adjustment_db",
    "comfort_volume",
    "safety_min_volume",
    "safety_max_volume",
    "private",
    "exclusive",
    "volume_mode",
    "max_sample_rate",
    "max_bit_depth",
  ] as const;
  for (const key of keys) {
    if (draft[key] !== initial[key]) {
      (patch as Record<string, unknown>)[key] = draft[key];
    }
  }
  const dspKeys = [
    "enabled",
    "preamp_db",
    "bass_db",
    "treble_db",
    "crossfeed",
  ] as const;
  if (dspKeys.some((key) => draft.dsp[key] !== initial.dsp[key])) {
    patch.dsp = { ...draft.dsp };
  }
  return patch;
}

/** Problems with a draft, in words, or `null` when it can be saved. */
export function zoneSettingsProblem(draft: ZoneSettingsDraft): string | null {
  const min = draft.safety_min_volume ?? 0;
  const max = draft.safety_max_volume ?? 100;
  if (min >= max) return "The safety minimum must be below the maximum.";
  if (
    draft.comfort_volume !== null &&
    (draft.comfort_volume < min || draft.comfort_volume > max)
  ) {
    return "The comfort limit must sit inside the safety limits.";
  }
  return null;
}

/** Where a zone's volume slider stops (the comfort limit, else the maximum). */
export function zoneSliderMax(volume: ZoneVolume): number {
  return Math.min(volume.comfort ?? volume.max, volume.max);
}

export type LevelBy = "track" | "album";

/**
 * How the queue item at `index` is leveled; `auto` uses the album while a
 * neighbour comes from the same album (an album playing in order).
 */
export function resolveLevelBy(
  mode: ZoneLevelingMode,
  songIds: readonly string[],
  index: number,
  albumOf: (songId: string) => string | null | undefined,
): LevelBy | null {
  if (mode === "off") return null;
  if (mode !== "auto") return mode;
  const id = songIds[index];
  const album = id ? albumOf(id) : null;
  if (!album) return "track";
  const same = (neighbour: string | undefined) =>
    neighbour !== undefined && albumOf(neighbour) === album;
  return same(songIds[index - 1]) || same(songIds[index + 1])
    ? "album"
    : "track";
}

/** Same limits as the server's leveling. */
export const LEVELING_MAX_BOOST_DB = 12;
export const LEVELING_MAX_CUT_DB = -24;
export const LEVELING_PEAK_CEILING_DBTP = -1;

/**
 * Gain (dB) bringing `loudness` to the target plus adjustment, limited so
 * the true peak stays at or below -1 dBTP. Mirrors the server.
 */
export function levelingGainDb(
  loudness: Loudness,
  targetLufs: number,
  adjustmentDb: number,
): number {
  const desired = targetLufs - loudness.integrated_lufs + adjustmentDb;
  const headroom = LEVELING_PEAK_CEILING_DBTP - loudness.true_peak_db;
  const gain = Math.max(
    LEVELING_MAX_CUT_DB,
    Math.min(LEVELING_MAX_BOOST_DB, Math.min(desired, headroom)),
  );
  return Math.round(gain * 10) / 10;
}
