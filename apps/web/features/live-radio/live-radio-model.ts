import {
  liveStationTrack,
  type LiveCategory,
  type LiveOrder,
  type LiveQualityTier,
  type LiveSearch,
  type LiveStation,
} from "@parson/music-sdk";
import type { QueueInput } from "@/features/player/player-model";

/** A station logo as an absolute URL (logos are relayed by the server). */
export function liveLogoSrc(
  logoUrl: string | null | undefined,
  baseURL: string,
): string | null {
  if (!logoUrl) return null;
  if (/^(https?:\/\/|data:image\/)/i.test(logoUrl)) return logoUrl;
  return `${baseURL.replace(/\/+$/, "")}${logoUrl.startsWith("/") ? "" : "/"}${logoUrl}`;
}

/** A station as a player queue item. */
export function stationQueueItem(
  station: LiveStation,
  baseURL: string,
): QueueInput {
  const track = liveStationTrack(
    station,
    liveLogoSrc(station.logo_url, baseURL),
  );
  return {
    song: track,
    album: track.album_object,
    artist: track.artist_object,
  };
}

/** Where a station's page lives. */
export const stationHref = (id: string) =>
  `/radio/station?id=${encodeURIComponent(id)}`;

function hash(value: string) {
  let result = 2166136261;
  for (const character of value) {
    result ^= character.codePointAt(0) ?? 0;
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}

/** Brand tones for logo-less station tiles: [lit corner, deep corner]. */
export const STATION_TONES: readonly [string, string][] = [
  ["#8a5638", "#1e1410"], // copper
  ["#4a6878", "#121a1f"], // steel
  ["#56704a", "#141a12"], // sage
  ["#86683f", "#1d1710"], // brass
  ["#7a5a54", "#1c1514"], // clay
  ["#4a4a52", "#111113"], // graphite
];

/** Initials and a stable gradient for stations without a logo. */
export function stationMonogram(name: string): {
  initials: string;
  from: string;
  to: string;
} {
  const words = name
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);
  const initials =
    words.length >= 2
      ? `${words[0]![0]}${words[1]![0]}`
      : (words[0]?.slice(0, 2) ?? "FM");
  const [from, to] = STATION_TONES[hash(name) % STATION_TONES.length]!;
  return { initials: initials.toUpperCase(), from, to };
}

const ORDERS: LiveOrder[] = [
  "popular",
  "trending",
  "votes",
  "bitrate",
  "name",
  "random",
];

/** Reads a search from the page's query string. */
export function searchFromParams(params: URLSearchParams): LiveSearch {
  const text = (key: string) => params.get(key)?.trim() || undefined;
  const order = text("order") as LiveOrder | undefined;
  const minBitrate = Number(params.get("min_bitrate"));
  return {
    name: text("q"),
    tag: text("tag"),
    country_code: text("country")?.toUpperCase(),
    language: text("language"),
    codec: text("codec"),
    lossless: params.get("lossless") === "1" || undefined,
    min_bitrate:
      Number.isFinite(minBitrate) && minBitrate > 0 ? minBitrate : undefined,
    order: order && ORDERS.includes(order) ? order : undefined,
  };
}

/** Writes a search to a query string (stable key order, no empties). */
export function paramsFromSearch(search: LiveSearch): string {
  const params = new URLSearchParams();
  if (search.name) params.set("q", search.name);
  if (search.tag) params.set("tag", search.tag);
  if (search.country_code) params.set("country", search.country_code);
  if (search.language) params.set("language", search.language);
  if (search.codec) params.set("codec", search.codec);
  if (search.lossless) params.set("lossless", "1");
  if (search.min_bitrate) params.set("min_bitrate", String(search.min_bitrate));
  if (search.order) params.set("order", search.order);
  return params.toString();
}

/** Whether a search asks for anything (otherwise the landing page shows). */
export function hasFilters(search: LiveSearch): boolean {
  return Boolean(
    search.order ||
    search.name ||
    search.tag ||
    search.country_code ||
    search.language ||
    search.codec ||
    search.lossless ||
    search.min_bitrate,
  );
}

export function categorySearch(category: LiveCategory): LiveSearch {
  return {
    tag: category.tag,
    lossless: category.lossless || undefined,
    min_bitrate: category.min_bitrate,
    order: category.order,
  };
}

/** A heading for a search's results. */
export function searchTitle(
  search: LiveSearch,
  categories: LiveCategory[],
  countryName: (code: string) => string | null,
): string {
  const category = categories.find(
    (item) =>
      (item.tag && item.tag === search.tag) ||
      (item.lossless && search.lossless && !search.tag) ||
      (item.min_bitrate &&
        item.min_bitrate === search.min_bitrate &&
        !search.tag) ||
      (item.group === "collections" &&
        !item.tag &&
        !item.lossless &&
        !item.min_bitrate &&
        item.order === search.order &&
        !search.tag &&
        !search.lossless &&
        !search.min_bitrate),
  );
  const parts: string[] = [];
  if (search.name) parts.push(`“${search.name}”`);
  if (category && !search.name) parts.push(category.label);
  else if (search.tag) parts.push(titleCase(search.tag));
  if (search.lossless && !category?.lossless) parts.push("Lossless");
  if (search.country_code) {
    parts.push(countryName(search.country_code) ?? search.country_code);
  }
  if (search.language) parts.push(titleCase(search.language));
  if (search.codec) parts.push(search.codec.toUpperCase());
  return parts.join(" · ") || "Stations";
}

function titleCase(value: string) {
  return value.replace(
    /(^|\s)(\p{L})/gu,
    (_, lead: string, letter: string) => `${lead}${letter.toUpperCase()}`,
  );
}

/** The listener's region from the browser language (`en-GB` -> `GB`). */
export function browserRegion(
  languages: readonly string[] | undefined,
): string | null {
  for (const language of languages ?? []) {
    const region = /^[a-z]{2,3}-([A-Z]{2})\b/i.exec(language)?.[1];
    if (region) return region.toUpperCase();
  }
  return null;
}

export const QUALITY_BADGE_CLASSES: Record<LiveQualityTier, string> = {
  // The brand's quality colours, as on the signal-path light.
  lossless:
    "border-quality-lossless/45 bg-quality-lossless/15 text-quality-lossless shadow-[0_0_12px_rgb(var(--mj-accent-rgb)/0.25)]",
  high: "border-quality-high/35 bg-quality-high/10 text-quality-high",
  standard: "border-white/12 bg-white/5 text-zinc-300",
  low: "border-quality-low/45 bg-quality-low/10 text-zinc-400",
  unknown: "border-white/8 bg-white/3 text-zinc-500",
};

/** Compact listener-facing numbers: `12.4k`. */
export function compactCount(value: number | null | undefined): string | null {
  if (!value || value < 1) return null;
  if (value < 1000) return String(value);
  if (value < 1_000_000) {
    const thousands = value / 1000;
    return `${thousands >= 100 ? Math.round(thousands) : thousands.toFixed(1).replace(/\.0$/, "")}k`;
  }
  return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

/** Moves one id within an ordered list (for reordering My Live Radio). */
export function moveId(ids: string[], from: number, to: number): string[] {
  if (from === to || from < 0 || to < 0 || from >= ids.length) return ids;
  const next = [...ids];
  const [moved] = next.splice(from, 1);
  next.splice(Math.min(to, next.length), 0, moved!);
  return next;
}
