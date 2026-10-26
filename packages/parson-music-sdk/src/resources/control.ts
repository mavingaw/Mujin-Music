import api from "../core/http";

/**
 * The controller API: what home automation (the Mujin Music integration
 * for Home Assistant, voice assistants, scripts) uses to browse the
 * library and start music in a zone without building queues itself.
 */

export type BrowseItemKind =
  | "library"
  | "directory"
  | "album"
  | "artist"
  | "playlist"
  | "genre"
  | "song"
  | "radio";

export interface BrowseItem {
  /** Pass back to `browseLibrary` or `playInZone`, e.g. `album:<id>`. */
  id: string;
  title: string;
  subtitle?: string;
  /** Artwork path for `/media/images/{path}`. */
  image?: string;
  kind: BrowseItemKind;
  playable: boolean;
  expandable: boolean;
  duration_ms?: number;
  /** Source format, e.g. "FLAC 24/96". */
  quality?: string;
}

export interface BrowseResponse {
  item: BrowseItem;
  children: BrowseItem[];
  /** Children before the server's per-response cut. */
  total: number;
}

export type PlayAction = "play" | "shuffle" | "next" | "add" | "radio";
export type SearchKind = "artist" | "album" | "song" | "playlist" | "genre";

export interface PlayInZoneRequest {
  item?: string;
  query?: string;
  kind?: SearchKind;
  action?: PlayAction;
}

export interface PlayInZoneResult {
  zone_id: string;
  item_id: string;
  title: string;
  action: PlayAction;
  songs: number;
}

export type TrackRating = "up" | "down";

export interface RatingResult {
  zone_id: string;
  song_id: string;
  rating: TrackRating;
  /** Thumbs down on the playing track skips it. */
  skipped: boolean;
}

const zonePath = (zoneId: string) => `/zones/${encodeURIComponent(zoneId)}`;

export async function browseLibrary(item = "root"): Promise<BrowseResponse> {
  return (await api.get<BrowseResponse>("/browse", { params: { item } })).data;
}

export async function searchLibraryItems(
  query: string,
): Promise<BrowseResponse> {
  return (await api.get<BrowseResponse>("/browse", { params: { query } })).data;
}

export async function playInZone(
  zoneId: string,
  request: PlayInZoneRequest,
): Promise<PlayInZoneResult> {
  return (
    await api.post<PlayInZoneResult>(`${zonePath(zoneId)}/play`, request, {
      timeout: 15_000,
    })
  ).data;
}

/** Starts a station seeded by `request` (library radio when empty). */
export async function startZoneRadio(
  zoneId: string,
  request: Omit<PlayInZoneRequest, "action"> = {},
): Promise<PlayInZoneResult> {
  return (
    await api.post<PlayInZoneResult>(
      `${zonePath(zoneId)}/radio`,
      { ...request, action: "radio" },
      { timeout: 15_000 },
    )
  ).data;
}

/** Thumbs up or down on a zone's playing track (or `songId`). */
export async function rateZoneTrack(
  zoneId: string,
  rating: TrackRating,
  songId?: string,
): Promise<RatingResult> {
  return (
    await api.post<RatingResult>(`${zonePath(zoneId)}/rating`, {
      rating,
      ...(songId ? { song_id: songId } : {}),
    })
  ).data;
}

/** The HACS custom repository that carries the Home Assistant integration. */
export const HOME_ASSISTANT_REPOSITORY =
  "https://github.com/mavingaw/parson-jin";
export const HOME_ASSISTANT_DOMAIN = "mujin_music";

export interface HomeAssistantConnection {
  host: string;
  port: number;
  ssl: boolean;
}

/**
 * The host and port to type into Home Assistant when discovery does not
 * find the server (the address this app reached it on).
 */
export function homeAssistantConnection(
  origin: string,
): HomeAssistantConnection | null {
  let url: URL;
  try {
    url = new URL(origin);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  const ssl = url.protocol === "https:";
  const host = url.hostname.replace(/^\[(.*)\]$/, "$1");
  return {
    host,
    port: url.port ? Number(url.port) : ssl ? 443 : 80,
    ssl,
  };
}

/** Loopback addresses that Home Assistant on another machine cannot reach. */
export function isLoopbackHost(host: string): boolean {
  const value = host.toLowerCase();
  return (
    value === "localhost" ||
    value === "::1" ||
    value.startsWith("127.") ||
    value.endsWith(".localhost")
  );
}

/** A ready-to-paste automation: morning radio at a gentle volume. */
export function homeAssistantAutomationExample(
  entityId = "media_player.living_room",
): string {
  return [
    "automation:",
    "  - alias: Good morning music",
    "    triggers:",
    "      - trigger: time",
    '        at: "07:30:00"',
    "    actions:",
    "      - action: mujin_music.start_radio",
    "        target:",
    `          entity_id: ${entityId}`,
    "        data:",
    "          query: Morning Jazz",
    "          kind: genre",
    "      - action: media_player.volume_set",
    "        target:",
    `          entity_id: ${entityId}`,
    "        data:",
    "          volume_level: 0.25",
  ].join("\n");
}
