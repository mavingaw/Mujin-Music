import type {
  LiveNowPlaying,
  LiveQualityTier,
  LiveStation,
  LiveStreamFormat,
  LiveTitle,
} from "../resources/live-radio";

/** Polling interval for now-playing titles while a station plays. */
export const LIVE_TITLE_POLL_MS = 10_000;

const regionNames = (() => {
  try {
    return new Intl.DisplayNames(["en"], {
      type: "region",
      fallback: "none",
    });
  } catch {
    return null;
  }
})();

/** `FR` -> `France`; falls back to the directory's own name. */
export function liveCountryName(
  code: string | null | undefined,
  fallback?: string | null,
): string | null {
  const upper = code?.trim().toUpperCase();
  if (upper && /^[A-Z]{2}$/.test(upper)) {
    try {
      const name = regionNames?.of(upper);
      if (name && name !== upper && name !== "Unknown Region") return name;
    } catch {}
  }
  const cleaned = fallback?.trim();
  if (!cleaned) return null;
  // radio-browser writes "The United States Of America".
  return cleaned.replace(/^The /, "");
}

function titleCase(value: string) {
  return value.replace(
    /(^|[\s/-])(\p{L})/gu,
    (_, lead: string, letter: string) => lead + letter.toUpperCase(),
  );
}

/** Tags that describe the stream, not the music (`aac`, `128k`, `hd`). */
const NOISE_TAGS =
  /^(mp3|aac\+?|ogg|opus|flac|hls|hd|hi-?fi|lossless|stereo|radio|fm|am|webradio|internet radio|\d+\s*k(bps)?|\d+)$/i;

/** Up to `count` tags, title-cased, for chips and subtitles. */
export function liveStationGenres(
  station: Pick<LiveStation, "tags">,
  count = 2,
): string[] {
  return station.tags
    .filter((tag) => tag.length <= 24 && !NOISE_TAGS.test(tag))
    .slice(0, count)
    .map(titleCase);
}

/** `Paris, France · Pop/Rock`: where a station is and what it plays. */
export function liveStationSubtitle(
  station: Pick<
    LiveStation,
    "region" | "country" | "country_code" | "tags" | "source"
  >,
): string {
  const country = liveCountryName(station.country_code, station.country);
  const place = [station.region, country]
    .filter((part): part is string => Boolean(part && part.trim()))
    .filter((part, index, parts) => parts.indexOf(part) === index)
    .join(", ");
  const genres = liveStationGenres(station).join(" / ");
  const parts = [place, genres].filter(Boolean);
  if (!parts.length) {
    return station.source === "custom" ? "Added by URL" : "Live radio";
  }
  return parts.join(" · ");
}

/** `Miles Davis — So What`, or the raw title. */
export function liveTitleText(title: LiveTitle | null | undefined): string {
  if (!title) return "";
  return title.artist ? `${title.artist} — ${title.title}` : title.title;
}

/** Shown under a playing station's name: its current title or place. */
export function liveNowPlayingLine(
  station: Pick<
    LiveStation,
    "region" | "country" | "country_code" | "tags" | "source"
  >,
  playing: Pick<LiveNowPlaying, "title"> | null | undefined,
): string {
  return liveTitleText(playing?.title) || liveStationSubtitle(station);
}

function rateText(rate: number) {
  const khz = rate / 1000;
  return `${Number.isInteger(khz) ? khz : khz.toFixed(1)} kHz`;
}

const CODEC_LABELS: Record<string, string> = {
  mp3: "MP3",
  aac: "AAC",
  flac: "FLAC",
  vorbis: "Ogg Vorbis",
  opus: "Opus",
  alac: "ALAC",
  pcm: "PCM",
};

export function liveCodecLabel(codec: string | null | undefined): string {
  if (!codec) return "Unknown";
  return CODEC_LABELS[codec.toLowerCase()] ?? codec.toUpperCase();
}

/** `AAC · 128 kbps · 48 kHz · stereo`. */
export function liveFormatText(
  format: Pick<
    LiveStreamFormat,
    "codec" | "bitrate_kbps" | "sample_rate" | "bit_depth" | "channels"
  >,
): string {
  const lossless = ["flac", "alac", "pcm"].includes(format.codec ?? "");
  const parts = [liveCodecLabel(format.codec)];
  if (format.bitrate_kbps && !lossless)
    parts.push(`${format.bitrate_kbps} kbps`);
  if (format.sample_rate) parts.push(rateText(format.sample_rate));
  if (format.bit_depth) parts.push(`${format.bit_depth}-bit`);
  if (format.channels === 1) parts.push("mono");
  if (format.channels === 2) parts.push("stereo");
  return parts.join(" · ");
}

export const LIVE_QUALITY_DESCRIPTIONS: Record<LiveQualityTier, string> = {
  lossless: "Lossless: the station sends every bit (FLAC).",
  high: "High-bitrate lossy: transparent for most listeners, but not lossless.",
  standard: "Typical lossy radio quality.",
  low: "Low bitrate: audible compression.",
  unknown: "The station does not say what it sends.",
};

/** What a player shows for a station: a song-like record. */
export interface LiveStationTrack {
  id: string;
  name: string;
  artist: string;
  duration: number;
  path: string;
  track_number: number;
  contributing_artists: string[];
  contributing_artist_ids: string[];
  album_object: { id: string; name: string; cover_url: string };
  artist_object: { id: string; name: string };
}

/**
 * A station as a player queue item: the station is the "track", its place
 * and genres the "artist", and its logo the cover. Duration 0 marks it live.
 */
export function liveStationTrack(
  station: Pick<
    LiveStation,
    | "song_id"
    | "name"
    | "region"
    | "country"
    | "country_code"
    | "tags"
    | "source"
  >,
  logoUrl: string | null,
): LiveStationTrack {
  const subtitle = liveStationSubtitle(station);
  return {
    id: station.song_id,
    name: station.name,
    artist: subtitle,
    duration: 0,
    path: "",
    track_number: 0,
    contributing_artists: [],
    contributing_artist_ids: [],
    album_object: { id: "", name: "Live radio", cover_url: logoUrl ?? "" },
    artist_object: { id: "", name: subtitle },
  };
}

/** Splits a free-text tag field into clean tags. */
export function parseTagInput(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/[,;/]/)
    .map((tag) => tag.trim().toLowerCase())
    .filter((tag) => tag && tag.length <= 40 && !seen.has(tag) && seen.add(tag))
    .slice(0, 12);
}

/** Whether a string looks like a stream URL a listener can add. */
export function looksLikeStreamUrl(value: string): boolean {
  try {
    const url = new URL(value.trim());
    return (
      (url.protocol === "http:" || url.protocol === "https:") &&
      Boolean(url.hostname) &&
      !url.username &&
      !url.password
    );
  } catch {
    return false;
  }
}
