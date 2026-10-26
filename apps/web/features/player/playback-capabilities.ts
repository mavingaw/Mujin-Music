import {
  detectPlaybackCapabilities,
  type PlaybackCapabilities,
} from "@parson/music-sdk";

/** Browsers resample anything above this; higher sources are fitted in-family. */
export const BROWSER_MAX_SAMPLE_RATE = 192_000;

let cached: PlaybackCapabilities | null | undefined;

/**
 * What this browser's media element decodes natively, detected once. `null`
 * outside a browser, which makes the server send original bytes.
 */
export function browserPlaybackCapabilities(): PlaybackCapabilities | null {
  if (cached !== undefined) return cached;
  if (typeof document === "undefined") return null;
  const element = document.createElement("audio");
  const detected = detectPlaybackCapabilities(
    (mime) => element.canPlayType(mime),
    { maxSampleRate: BROWSER_MAX_SAMPLE_RATE },
  );
  cached = detected.accept.length > 0 ? detected : null;
  return cached;
}
