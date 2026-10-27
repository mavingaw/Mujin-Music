import {
  detectPlaybackCapabilities,
  type PlaybackCapabilities,
} from "@parson/music-sdk";
import type { AudioProperties } from "@parson/music-sdk/types";

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

/**
 * The PCM rate the server streams this file at for a browser: the source
 * rate, DSD as PCM at its rate / 16, fitted under the browser's limit within
 * the same rate family (the server's delivery plan does the same).
 */
export function expectedStreamSampleRate(
  audio: AudioProperties | null | undefined,
): number | null {
  const rate = audio?.sample_rate;
  if (!rate || rate <= 0) return null;
  const dsd = audio.codec === "dsd" || audio.codec === "dst";
  let pcm = dsd ? Math.round(rate / 16) : rate;
  while (pcm > BROWSER_MAX_SAMPLE_RATE && pcm % 2 === 0 && pcm / 2 >= 8_000) {
    pcm /= 2;
  }
  return Math.min(pcm, BROWSER_MAX_SAMPLE_RATE);
}
