import api, { getApiWebSocketURL } from "../core/http";

/**
 * Zones are every place music can play: each running Parson app (web,
 * desktop, mobile), an active Chromecast session, each Sonos room or group,
 * and linked groups of zones. The server keeps the list; clients register
 * themselves, pick a zone to control, transfer queues and group zones.
 */
export type ZoneKind =
  "web" | "desktop" | "mobile" | "chromecast" | "sonos" | "linked";

export type ZoneStatus =
  "playing" | "paused" | "buffering" | "stopped" | "idle" | "unavailable";

export type ZoneSource = "none" | "parson" | "external";

export interface ZoneVolume {
  level: number;
  muted: boolean;
  min: number;
  max: number;
  step: number;
}

export interface ZoneGroupMember {
  id: string;
  name: string;
  kind: ZoneKind;
  volume: ZoneVolume | null;
}

export interface ZoneGroupInfo {
  id: string;
  /** True only for native groups (Sonos) that play sample-synced. */
  synced: boolean;
  sync_note: string;
  members: ZoneGroupMember[];
}

export interface ZoneCapabilities {
  transport: boolean;
  seek: boolean;
  volume: boolean;
  mute: boolean;
  queue: boolean;
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
  ecosystem: "parson" | "chromecast" | "sonos" | "linked";
  name: string;
  device: string;
  status: ZoneStatus;
  source: ZoneSource;
  now_playing: ZoneNowPlaying | null;
  queue: { song_ids: string[]; current_position: number };
  position_ms: number;
  duration_ms: number;
  volume: ZoneVolume | null;
  group: ZoneGroupInfo | null;
  grouped_into: string | null;
  capabilities: ZoneCapabilities;
  sleep_timer: { ends_at_ms: number; finishing_track: boolean } | null;
  notes: string[];
}

export interface ZoneList {
  zones: Zone[];
  server_time_ms: number;
  sonos: { enabled: boolean; players: number; last_error: string | null };
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
    };

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

export async function getZones(): Promise<ZoneList> {
  const response = await api.get<ZoneList>("/zones", { timeout: 5_000 });
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
  await api.post(`${zonePath(zoneId)}/commands`, command, { timeout: 8_000 });
}

/** Moves the queue, track and position from one zone to another. */
export async function transferZone(
  from: string,
  to: string,
): Promise<{ zone: Zone | null }> {
  const response = await api.post<{ zone: Zone | null }>(
    "/zones/transfer",
    { from, to },
    { timeout: 30_000 },
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
    { timeout: 15_000 },
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
  settings: { name?: string | null; max_volume?: number | null },
): Promise<{ name: string | null; max_volume: number | null }> {
  const response = await api.patch(`${zonePath(zoneId)}/settings`, settings);
  return response.data;
}

/** Pauses the zone after `minutes`; `null` cancels the timer. */
export async function setZoneSleepTimer(
  zoneId: string,
  minutes: number | null,
): Promise<void> {
  await api.post(`${zonePath(zoneId)}/sleep`, { minutes });
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
  linked: 2,
  sonos: 3,
  chromecast: 4,
};

/** Zones ordered for a picker: this device first, then apps, groups, Sonos, Cast. */
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
    case "linked":
      return "Linked zones";
  }
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
