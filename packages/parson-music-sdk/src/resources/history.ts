import api from "../core/http";
import type { FocusQualityBadge } from "./focus";

/** Zone filter value for plays recorded before zones were tracked. */
export const UNKNOWN_HISTORY_ZONE = "none";

export interface HistoryFilter {
  /** Start of the period (ISO 8601 instant). */
  from?: string;
  /** End of the period, exclusive. */
  to?: string;
  /** A zone id, or {@link UNKNOWN_HISTORY_ZONE}. */
  zone?: string;
  /** `manual`, `generated` (Radio), ... */
  source?: string;
  device?: string;
  artistId?: string;
  albumId?: string;
  songId?: string;
  q?: string;
  /** Viewer's offset from UTC in minutes (east positive). */
  tz?: number;
}

export interface HistoryEntry {
  id: number;
  songId: string;
  /** ISO 8601 instant, UTC. */
  listenedAt: string;
  title: string;
  artistId?: string | null;
  artistName: string;
  albumId?: string | null;
  albumName?: string | null;
  coverPath?: string | null;
  durationSeconds: number;
  zoneId?: string | null;
  zoneName?: string | null;
  source?: string | null;
  device?: string | null;
  /** The track is still in the library. */
  available: boolean;
  quality?: FocusQualityBadge | null;
}

export interface HistoryTimeline {
  items: HistoryEntry[];
  nextBeforeId?: number | null;
}

export type HistoryBucket = "day" | "week" | "month" | "year";

export interface HistoryTotals {
  plays: number;
  listenedSeconds: number;
  tracks: number;
  artists: number;
  albums: number;
  activeDays: number;
  firstPlay?: string | null;
  lastPlay?: string | null;
}

export interface HistoryRankedItem {
  id: string;
  name: string;
  detail?: string | null;
  detailId?: string | null;
  imagePath?: string | null;
  plays: number;
  listenedSeconds: number;
  available: boolean;
}

export interface HistoryBucketCount {
  bucket: string;
  plays: number;
  listenedSeconds: number;
}

export interface HistoryGroupCount {
  value: string;
  label?: string | null;
  plays: number;
}

export interface HistoryStats {
  totals: HistoryTotals;
  /** The period of the same length just before, when both ends are set. */
  previous?: HistoryTotals | null;
  bucket: HistoryBucket;
  timeline: HistoryBucketCount[];
  /** Plays per hour of day, 0-23, in the viewer's time zone. */
  hours: number[];
  /** Plays per weekday, Monday first. */
  weekdays: number[];
  topArtists: HistoryRankedItem[];
  topAlbums: HistoryRankedItem[];
  topTracks: HistoryRankedItem[];
  zones: HistoryGroupCount[];
  sources: HistoryGroupCount[];
}

export interface HistoryFilters {
  zones: HistoryGroupCount[];
  sources: HistoryGroupCount[];
  devices: HistoryGroupCount[];
  firstPlay?: string | null;
  total: number;
}

/** Query parameters for a filter, in the server's snake_case. */
export function historyParams(
  filter: HistoryFilter,
): Record<string, string | number> {
  const params: Record<string, string | number> = {};
  if (filter.from) params.from = filter.from;
  if (filter.to) params.to = filter.to;
  if (filter.zone) params.zone = filter.zone;
  if (filter.source) params.source = filter.source;
  if (filter.device) params.device = filter.device;
  if (filter.artistId) params.artist_id = filter.artistId;
  if (filter.albumId) params.album_id = filter.albumId;
  if (filter.songId) params.song_id = filter.songId;
  if (filter.q?.trim()) params.q = filter.q.trim();
  if (filter.tz !== undefined && Number.isFinite(filter.tz)) {
    params.tz = Math.round(filter.tz);
  }
  return params;
}

/** This device's offset from UTC in minutes, east positive. */
export function localTimezoneOffset(date: Date = new Date()): number {
  return -date.getTimezoneOffset();
}

export async function getHistoryTimeline(
  filter: HistoryFilter = {},
  page: { beforeId?: number | null; limit?: number } = {},
): Promise<HistoryTimeline> {
  const params = historyParams(filter);
  if (page.beforeId) params.before_id = page.beforeId;
  if (page.limit) params.limit = page.limit;
  const response = await api.get<HistoryTimeline>(
    "/users/me/history/timeline",
    { params },
  );
  return {
    items: Array.isArray(response.data?.items) ? response.data.items : [],
    nextBeforeId: response.data?.nextBeforeId ?? null,
  };
}

export async function getHistoryStats(
  filter: HistoryFilter = {},
  options: { bucket?: HistoryBucket; top?: number } = {},
): Promise<HistoryStats> {
  const params = historyParams(filter);
  if (options.bucket) params.bucket = options.bucket;
  if (options.top) params.top = options.top;
  const response = await api.get<HistoryStats>("/users/me/history/stats", {
    params,
  });
  return response.data;
}

export async function getHistoryFilters(): Promise<HistoryFilters> {
  const response = await api.get<HistoryFilters>("/users/me/history/filters");
  return {
    zones: response.data?.zones ?? [],
    sources: response.data?.sources ?? [],
    devices: response.data?.devices ?? [],
    firstPlay: response.data?.firstPlay ?? null,
    total: response.data?.total ?? 0,
  };
}

/** The filtered history as CSV text (UTF-8, RFC 4180). */
export async function exportHistoryCsv(
  filter: HistoryFilter = {},
): Promise<string> {
  const response = await api.get<string>("/users/me/history/export.csv", {
    params: historyParams(filter),
    responseType: "text",
    timeout: 60_000,
  });
  return typeof response.data === "string" ? response.data : "";
}

export async function deleteHistoryEntry(id: number): Promise<void> {
  await api.delete(`/users/me/history/entries/${id}`);
}

export type HistoryPeriodPreset =
  "today" | "week" | "month" | "year" | "last-year" | "all";

export const HISTORY_PERIODS: { value: HistoryPeriodPreset; label: string }[] =
  [
    { value: "today", label: "Today" },
    { value: "week", label: "7 days" },
    { value: "month", label: "30 days" },
    { value: "year", label: "This year" },
    { value: "last-year", label: "Last year" },
    { value: "all", label: "All time" },
  ];

export interface HistoryPeriod {
  from?: string;
  to?: string;
  bucket: HistoryBucket;
}

/**
 * The instants a preset covers in the viewer's local time, and the bucket
 * that charts it well. Rolling periods end at the next local midnight so
 * today is always included.
 */
export function historyPeriod(
  preset: HistoryPeriodPreset,
  now: Date = new Date(),
): HistoryPeriod {
  const midnight = (year: number, month: number, day: number) =>
    new Date(year, month, day);
  const tomorrow = midnight(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + 1,
  );
  const daysBack = (days: number) =>
    midnight(now.getFullYear(), now.getMonth(), now.getDate() - days + 1);
  switch (preset) {
    case "today":
      return {
        from: daysBack(1).toISOString(),
        to: tomorrow.toISOString(),
        bucket: "day",
      };
    case "week":
      return {
        from: daysBack(7).toISOString(),
        to: tomorrow.toISOString(),
        bucket: "day",
      };
    case "month":
      return {
        from: daysBack(30).toISOString(),
        to: tomorrow.toISOString(),
        bucket: "day",
      };
    case "year":
      return {
        from: midnight(now.getFullYear(), 0, 1).toISOString(),
        to: midnight(now.getFullYear() + 1, 0, 1).toISOString(),
        bucket: "month",
      };
    case "last-year":
      return {
        from: midnight(now.getFullYear() - 1, 0, 1).toISOString(),
        to: midnight(now.getFullYear(), 0, 1).toISOString(),
        bucket: "month",
      };
    case "all":
      return { bucket: "month" };
  }
}

export interface HistoryDay {
  /** Local calendar day, `YYYY-MM-DD`. */
  key: string;
  label: string;
  items: HistoryEntry[];
  listenedSeconds: number;
}

function localDayKey(date: Date) {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** "Today", "Yesterday", "Monday", "Sunday 12 October", "12 October 2025". */
export function historyDayLabel(key: string, now: Date = new Date()): string {
  const [year = 1970, month = 1, day = 1] = key.split("-").map(Number);
  const date = new Date(year, month - 1, day);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days > 1 && days < 7) {
    return date.toLocaleDateString("en-GB", { weekday: "long" });
  }
  return date.toLocaleDateString("en-GB", {
    weekday: year === now.getFullYear() ? "long" : undefined,
    day: "numeric",
    month: "long",
    year: year === now.getFullYear() ? undefined : "numeric",
  });
}

/** Groups newest-first entries by local day, keeping their order. */
export function groupHistoryByDay(
  items: HistoryEntry[],
  now: Date = new Date(),
): HistoryDay[] {
  const days: HistoryDay[] = [];
  for (const item of items) {
    const played = new Date(item.listenedAt);
    if (Number.isNaN(played.getTime())) continue;
    const key = localDayKey(played);
    let day = days[days.length - 1];
    if (!day || day.key !== key) {
      day = {
        key,
        label: historyDayLabel(key, now),
        items: [],
        listenedSeconds: 0,
      };
      days.push(day);
    }
    day.items.push(item);
    day.listenedSeconds += Math.max(0, item.durationSeconds || 0);
  }
  return days;
}

/** "45 min", "3 h 20 min", "2 days 4 h". */
export function formatListeningTime(seconds: number): string {
  const minutes = Math.round(Math.max(0, seconds) / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) {
    const rest = minutes % 60;
    return rest ? `${hours} h ${rest} min` : `${hours} h`;
  }
  const days = Math.floor(hours / 24);
  const rest = hours % 24;
  return rest ? `${days} days ${rest} h` : `${days} days`;
}

/** Relative change from the previous period, or null without a base. */
export function periodChange(current: number, previous?: number | null) {
  if (previous === undefined || previous === null || previous <= 0) {
    return null;
  }
  return (current - previous) / previous;
}

/** Human label for a play's source. */
export function historySourceLabel(source?: string | null): string {
  switch (source) {
    case "manual":
      return "Your picks";
    case "generated":
    case "radio":
      return "Radio";
    case "zone":
      return "Remote";
    case null:
    case undefined:
    case "unknown":
      return "Unknown";
    default:
      return source.charAt(0).toUpperCase() + source.slice(1);
  }
}

/** Human label for a play's zone. */
export function historyZoneLabel(
  zone: Pick<HistoryGroupCount, "value" | "label">,
): string {
  if (zone.value === UNKNOWN_HISTORY_ZONE) return "Not recorded";
  return zone.label?.trim() || zone.value;
}

/** Local time of day for a play: "21:04". */
export function historyTimeLabel(listenedAt: string): string {
  const date = new Date(listenedAt);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
}
