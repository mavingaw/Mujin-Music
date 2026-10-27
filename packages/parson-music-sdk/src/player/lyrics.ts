/**
 * Timed lyrics shared by the web and phone players: LRC parsing, the line
 * that is sung now, how strongly each line shows, and the words for where
 * lyrics came from or why there are none.
 */

/** Where a track's lyrics came from (`source` in `GET /lyrics/{song}`). */
export type LyricsSourceKind =
  "sidecar_lrc" | "embedded_sylt" | "embedded_lrc" | "embedded_text" | "lrclib";

export type TimedLyricLine = {
  /** Seconds from the start of the track. */
  time: number;
  /** The words; empty for an instrumental break. */
  text: string;
};

const MAX_LINES = 5_000;

/** `mm:ss`, `mm:ss.xx`, `mm:ss.xxx`, `mm:ss:xx` or `h:mm:ss.xx` in seconds. */
export function parseLrcStamp(stamp: string): number | null {
  const value = stamp.trim();
  let match = /^(\d{1,3}):(\d{2}):(\d{2})$/.exec(value);
  if (match) {
    const seconds = Number(match[2]);
    if (seconds >= 60) return null;
    return Number(match[1]) * 60 + seconds + Number(match[3]) / 100;
  }
  match = /^(?:(\d+):)?(\d{1,3}):(\d{1,2})(?:\.(\d{1,3}))?$/.exec(value);
  if (!match) return null;
  const hours = Number(match[1] ?? 0);
  const minutes = Number(match[2]);
  const seconds = Number(match[3]);
  if (seconds >= 60 || (match[1] !== undefined && minutes >= 60)) return null;
  const fraction = match[4] ? Number(`0.${match[4]}`) : 0;
  return hours * 3600 + minutes * 60 + seconds + fraction;
}

function stripWordStamps(text: string) {
  return text
    .replace(/<([^<>]*)>/g, (whole, inner: string) =>
      parseLrcStamp(inner) === null ? whole : "",
    )
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * Parses LRC into timed lines sorted by time. Handles several stamps on one
 * line, `[offset:±ms]`, header tags (`[ar:]`, `[ti:]`), enhanced-LRC word
 * stamps and blank timed lines (instrumental breaks).
 */
export function parseLrc(value?: string | null): TimedLyricLine[] {
  if (!value) return [];
  let offsetSeconds = 0;
  const lines: TimedLyricLine[] = [];
  for (const raw of value.split(/\r?\n/)) {
    let rest = raw.replace(/^﻿/, "").trim();
    const stamps: number[] = [];
    while (rest.startsWith("[")) {
      const close = rest.indexOf("]");
      if (close < 0) break;
      const tag = rest.slice(1, close);
      const stamp = parseLrcStamp(tag);
      if (stamp !== null) {
        stamps.push(stamp);
      } else {
        const separator = tag.indexOf(":");
        if (separator < 0) break;
        if (tag.slice(0, separator).trim().toLowerCase() === "offset") {
          const offset = Number(tag.slice(separator + 1).trim());
          offsetSeconds = Number.isFinite(offset) ? offset / 1000 : 0;
        }
      }
      rest = rest.slice(close + 1).trimStart();
    }
    if (!stamps.length) continue;
    const text = stripWordStamps(rest);
    for (const stamp of stamps) {
      lines.push({ time: Math.max(0, stamp - offsetSeconds), text });
      if (lines.length >= MAX_LINES) break;
    }
    if (lines.length >= MAX_LINES) break;
  }
  return lines
    .map((line, index) => ({ line, index }))
    .sort((a, b) => a.line.time - b.line.time || a.index - b.index)
    .map(({ line }) => line);
}

/** The line being sung at `time` (seconds), or -1 before the first. */
export function activeLyricIndex(lines: TimedLyricLine[], time: number) {
  let low = 0;
  let high = lines.length - 1;
  let found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (lines[middle]!.time <= time) {
      found = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }
  return found;
}

export type LyricLineState = "past" | "active" | "next" | "upcoming";

/**
 * How a line shows around the sung one: the sung line at full strength,
 * the lines after it fading out with distance, sung lines dimmed.
 */
export function lyricLineEmphasis(
  index: number,
  active: number,
): { state: LyricLineState; opacity: number } {
  if (index === active) return { state: "active", opacity: 1 };
  if (index < active) return { state: "past", opacity: 0.22 };
  const distance = index - Math.max(active, -1);
  if (distance === 1) return { state: "next", opacity: 0.48 };
  return {
    state: "upcoming",
    opacity: Math.max(0.14, 0.48 - (distance - 1) * 0.09),
  };
}

/** Plain words for where the lyrics came from. */
export function lyricsSourceLabel(source?: LyricsSourceKind | null) {
  switch (source) {
    case "sidecar_lrc":
      return "LRC file beside the track";
    case "embedded_sylt":
      return "Synchronised lyrics in the file (SYLT)";
    case "embedded_lrc":
      return "LRC embedded in the file";
    case "embedded_text":
      return "Lyrics tag in the file";
    case "lrclib":
      return "LRCLIB";
    default:
      return null;
  }
}

/** Why a lyrics request found nothing. */
export type LyricsFailure =
  /** Nothing in the file, nothing online. */
  | "missing"
  /** Nothing in the file, and online lookups are off. */
  | "offline"
  /** The online source did not answer (503). */
  | "unavailable"
  /** Anything else: the server could not be reached. */
  | "failed";

export function lyricsFailureKind(error: unknown): LyricsFailure {
  if (!error || typeof error !== "object" || !("response" in error)) {
    return "failed";
  }
  const response = (error as { response?: unknown }).response;
  if (!response || typeof response !== "object") return "failed";
  const status = (response as { status?: unknown }).status;
  const data = (response as { data?: unknown }).data;
  const code =
    data && typeof data === "object" && "code" in data
      ? (data as { code?: unknown }).code
      : undefined;
  if (code === "lyrics_not_found") return "missing";
  if (code === "lyrics_not_found_offline") return "offline";
  if (status === 503) return "unavailable";
  return "failed";
}

/** The headline and explanation for a lyrics view with nothing to show. */
export function lyricsEmptyMessage(
  failure: LyricsFailure | null,
  instrumental = false,
): { title: string; detail: string; retry: boolean } {
  if (instrumental) {
    return {
      title: "Instrumental",
      detail: "This track has no words to follow.",
      retry: false,
    };
  }
  switch (failure) {
    case "offline":
      return {
        title: "No lyrics for this track",
        detail:
          "Nothing in the file or beside it. Online lookups are off: turn on online metadata in Settings to search LRCLIB.",
        retry: false,
      };
    case "unavailable":
      return {
        title: "No lyrics for this track",
        detail:
          "The file has none, and the online lyrics source (LRCLIB) is not answering right now (503). Playback is unaffected.",
        retry: true,
      };
    case "failed":
      return {
        title: "Lyrics could not be loaded",
        detail: "The server did not answer. Playback is unaffected.",
        retry: true,
      };
    default:
      return {
        title: "No lyrics for this track",
        detail:
          "Checked the file's tags, an LRC file beside it and LRCLIB. Add a .lrc file next to the track to see timed lyrics.",
        retry: false,
      };
  }
}
