/** Format tokens the server understands in `accept` (see `OutputCapabilities`). */
export type PlaybackFormatToken =
  | "flac"
  | "alac"
  | "aac"
  | "mp3"
  | "vorbis"
  | "opus"
  | "wav"
  | "wav_float"
  | "aiff"
  | "wavpack"
  | "ape"
  | "dsf"
  | "dff";

/** How a player wants DSD: converted to PCM, wrapped as DoP, or as original bytes. */
export type DsdPlayback = "pcm" | "dop" | "native";

/** What a player decodes, sent with stream requests so the server picks the best delivery. */
export interface PlaybackCapabilities {
  accept: PlaybackFormatToken[];
  maxSampleRate?: number;
  maxBitDepth?: number;
  maxChannels?: number;
  dsd?: DsdPlayback;
}

/** MIME probes for `HTMLMediaElement.canPlayType`, most specific first. */
export const PLAYBACK_FORMAT_PROBES: ReadonlyArray<
  readonly [PlaybackFormatToken, readonly string[]]
> = [
  ["flac", ["audio/flac", "audio/x-flac"]],
  ["alac", ['audio/mp4; codecs="alac"']],
  ["aac", ['audio/mp4; codecs="mp4a.40.2"', "audio/aac"]],
  ["mp3", ["audio/mpeg"]],
  ["vorbis", ['audio/ogg; codecs="vorbis"']],
  ["opus", ['audio/ogg; codecs="opus"', 'audio/webm; codecs="opus"']],
  ["wav", ['audio/wav; codecs="1"', "audio/wav"]],
  ["wav_float", ['audio/wav; codecs="3"']],
  ["aiff", ["audio/aiff", "audio/x-aiff"]],
];

/**
 * Detects what a media element decodes. Only "probably" counts for codec
 * strings; plain container probes also accept "maybe".
 */
export function detectPlaybackCapabilities(
  canPlayType: (mime: string) => string,
  limits: Omit<PlaybackCapabilities, "accept"> = {},
): PlaybackCapabilities {
  const accept: PlaybackFormatToken[] = [];
  for (const [token, mimes] of PLAYBACK_FORMAT_PROBES) {
    const supported = mimes.some((mime) => {
      let answer = "";
      try {
        answer = canPlayType(mime);
      } catch {
        answer = "";
      }
      return mime.includes("codecs=")
        ? answer === "probably"
        : answer === "probably" || answer === "maybe";
    });
    if (supported) accept.push(token);
  }
  return { accept, ...limits };
}

/** Adds capability parameters to a stream or delivery URL. */
export function applyPlaybackCapabilities(
  url: URL,
  capabilities: PlaybackCapabilities | null | undefined,
) {
  if (!capabilities || capabilities.accept.length === 0) return url;
  url.searchParams.set("accept", capabilities.accept.join(","));
  if (capabilities.maxSampleRate)
    url.searchParams.set("max_sample_rate", String(capabilities.maxSampleRate));
  if (capabilities.maxBitDepth)
    url.searchParams.set("max_bit_depth", String(capabilities.maxBitDepth));
  if (capabilities.maxChannels)
    url.searchParams.set("max_channels", String(capabilities.maxChannels));
  if (capabilities.dsd && capabilities.dsd !== "pcm")
    url.searchParams.set("dsd", capabilities.dsd);
  return url;
}

/** Query parameters for capability-aware requests through the SDK client. */
export function playbackCapabilityParams(
  capabilities: PlaybackCapabilities | null | undefined,
): Record<string, string> {
  const url = applyPlaybackCapabilities(
    new URL("http://parson.invalid/"),
    capabilities,
  );
  return Object.fromEntries(url.searchParams.entries());
}

/** One server-side processing stage, as reported by `/media/songs/{id}/delivery`. */
export interface SignalStep {
  kind:
    | "source"
    | "passthrough"
    | "decode"
    | "dsd_to_pcm"
    | "dop"
    | "resample"
    | "requantize"
    | "downmix"
    | "effect"
    | "encode"
    | (string & {});
  description: string;
  quality: "bit_perfect" | "lossless" | "high" | "enhanced" | "lossy";
}

export interface StreamFormat {
  name: string;
  codec: string | null;
  container: string | null;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
  bitrate_kbps: number | null;
  lossless: boolean | null;
}

export type DeliveryMode =
  "original" | "lossless_conversion" | "dop" | "lossy_transcode";

export type DeliveryQuality =
  "bit_perfect" | "lossless" | "high_quality" | "lossy";

/** How the server delivers a track to a player, and why. */
export interface DeliveryPlan {
  mode: DeliveryMode;
  content_type: string;
  source: StreamFormat;
  output: StreamFormat;
  steps: SignalStep[];
  quality: DeliveryQuality;
  bit_perfect: boolean;
  gapless: boolean;
  reason: string;
}

export interface SongDelivery {
  song_id: string;
  plan: DeliveryPlan;
}

/** Human label for a delivery quality. */
export function deliveryQualityLabel(quality: DeliveryQuality) {
  switch (quality) {
    case "bit_perfect":
      return "Bit-perfect";
    case "lossless":
      return "Lossless";
    case "high_quality":
      return "High quality";
    default:
      return "Lossy";
  }
}
