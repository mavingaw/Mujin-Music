import type { DisplayState, TimedLyricLine } from "@parson/music-sdk";

/**
 * What a display screen shows:
 * - `pairing`: no key yet; a code to approve from a signed-in app;
 * - `connecting`: a key, no answer yet;
 * - `idle`: paired, nothing plays (a clock);
 * - `missing`: the zone it is bound to is off or private;
 * - `lyrics`: a track with timed lyrics, and lyrics are on;
 * - `info`: a track, shown as artwork, photo and facts.
 */
export type DisplayMode =
  "pairing" | "connecting" | "idle" | "missing" | "lyrics" | "info";

export function displayMode({
  hasKey,
  state,
  timedLines,
  lyricsHidden,
}: {
  hasKey: boolean;
  state: DisplayState | null;
  timedLines: readonly TimedLyricLine[];
  /** The viewer switched lyrics off on this screen (L or the toolbar). */
  lyricsHidden: boolean;
}): DisplayMode {
  if (!hasKey) return "pairing";
  if (!state) return "connecting";
  if (state.status === "zone_missing") return "missing";
  if (state.status !== "playing" || !state.track) return "idle";
  const wordsTimed = timedLines.some((line) => line.text.trim().length > 0);
  if (
    wordsTimed &&
    !lyricsHidden &&
    state.display.lyrics !== "off" &&
    !state.track.live
  ) {
    return "lyrics";
  }
  return "info";
}

/** The clock in the corner: `21:47` or `9:47 PM`, as the screen's locale has it. */
export function clockText(date: Date, locale?: string): string {
  return new Intl.DateTimeFormat(locale, {
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

/** `m:ss` until a pairing code expires. */
export function pairingCountdown(expiresAtMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.ceil((expiresAtMs - nowMs) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** Waveform bars for a seek bar `widthPx` wide: about one per 10 px. */
export function waveformBarCount(widthPx: number): number {
  if (!Number.isFinite(widthPx) || widthPx <= 0) return 120;
  return Math.max(48, Math.min(220, Math.floor(widthPx / 10)));
}

/** A first name for a screen, from what its browser says it is. */
export function suggestDisplayName(userAgent: string): string {
  const agent = userAgent.toLowerCase();
  if (agent.includes("crkey")) return "Chromecast";
  if (
    /smart-?tv|tizen|web0s|webos|bravia|netcast|hbbtv|aftb|aftm|afts|aftt|android tv|googletv|appletv/.test(
      agent,
    )
  ) {
    return "TV";
  }
  if (
    agent.includes("ipad") ||
    (agent.includes("android") && !agent.includes("mobile"))
  )
    return "Tablet";
  if (agent.includes("iphone") || agent.includes("mobile")) return "Phone";
  return "Browser";
}

/**
 * Lines worth rendering around the sung one: a few behind it, more ahead,
 * so a long song never puts hundreds of nodes on a TV browser.
 */
export function lyricsWindow(
  length: number,
  active: number,
  before = 3,
  after = 7,
): { start: number; end: number } {
  if (length <= 0) return { start: 0, end: 0 };
  const anchor = Math.max(0, active);
  return {
    start: Math.max(0, anchor - before),
    end: Math.min(length, anchor + after + 1),
  };
}

/** Reads a stored value, tolerating browsers that refuse storage. */
export function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeStored(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // A private window: the screen pairs again next time.
  }
}
