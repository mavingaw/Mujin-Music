import type { AudioProperties } from "../domain/types";

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

/** Quality tier for badges: DSD, hi-res PCM, CD-quality lossless, or lossy. */
export type AudioQualityTier =
  "dsd" | "hi_res" | "lossless" | "lossy" | "unknown";

export interface AudioFormatSummary {
  /** e.g. `FLAC`, `WavPack`, `DSD64`, `MP3`. */
  name: string;
  /** e.g. `24-bit / 96 kHz`, `2.8224 MHz`, `320 kbps`. */
  resolution: string | null;
  /** Compact badge text, e.g. `24/96`, `DSD64`, `320k`. */
  badge: string;
  /** e.g. `stereo`, `5.1`; null when unknown. */
  channels: string | null;
  tier: AudioQualityTier;
}

const CODEC_NAMES: Record<string, string> = {
  flac: "FLAC",
  alac: "ALAC",
  wavpack: "WavPack",
  ape: "Monkey's Audio",
  tak: "TAK",
  tta: "True Audio",
  mp3: "MP3",
  aac: "AAC",
  vorbis: "Ogg Vorbis",
  opus: "Opus",
  musepack: "Musepack",
  wma: "WMA",
  wma_pro: "WMA Pro",
  wma_lossless: "WMA Lossless",
  speex: "Speex",
  ac3: "Dolby Digital",
  eac3: "Dolby Digital Plus",
  truehd: "Dolby TrueHD",
  dts: "DTS",
};

const CONTAINER_PCM_NAMES: Record<string, string> = {
  aiff: "AIFF",
  w64: "Wave64",
  caf: "CAF",
};

export function channelLayoutName(channels: number | null | undefined) {
  if (!channels) return null;
  return (
    (
      {
        1: "mono",
        2: "stereo",
        3: "3.0",
        4: "4.0",
        5: "5.0",
        6: "5.1",
        7: "6.1",
        8: "7.1",
      } as Record<number, string>
    )[channels] ?? `${channels} channels`
  );
}

export function formatSampleRate(rate: number) {
  return rate % 1000 === 0
    ? `${rate / 1000} kHz`
    : `${(rate / 1000).toFixed(1)} kHz`;
}

/** DSD rate name (`DSD64`, `DSD128`, ...) for a 1-bit rate in Hz. */
export function dsdRateName(rate: number | null | undefined) {
  if (!rate || rate < 2_000_000) return null;
  const base = rate % 48_000 === 0 && rate % 44_100 !== 0 ? 48_000 : 44_100;
  return `DSD${Math.round(rate / base)}`;
}

/** Summarises a track's technical properties for badges and the signal path. */
export function describeAudioFormat(
  audio: AudioProperties | null | undefined,
): AudioFormatSummary | null {
  if (!audio || (!audio.codec && !audio.sample_rate)) return null;
  const codec = audio.codec ?? "";
  const channels = channelLayoutName(audio.channels);
  if (codec === "dsd" || codec === "dst") {
    const name = dsdRateName(audio.sample_rate) ?? "DSD";
    return {
      name,
      resolution: audio.sample_rate
        ? `${(audio.sample_rate / 1_000_000).toFixed(4)} MHz`
        : null,
      badge: name,
      channels,
      tier: "dsd",
    };
  }
  const name =
    codec === "pcm" || codec === "pcm_float"
      ? (CONTAINER_PCM_NAMES[audio.container ?? ""] ?? "WAV")
      : (CODEC_NAMES[codec] ?? (codec ? codec.toUpperCase() : "Audio"));
  const rate = audio.sample_rate ?? null;
  const depth = audio.bit_depth ?? null;
  if (audio.lossless === false) {
    const kbps = audio.bitrate_kbps ?? null;
    return {
      name,
      resolution: kbps ? `${kbps} kbps` : rate ? formatSampleRate(rate) : null,
      badge: kbps ? `${kbps}k` : name,
      channels,
      tier: "lossy",
    };
  }
  const hiRes = (depth ?? 0) > 16 || (rate ?? 0) > 48_000;
  const resolution =
    depth && rate
      ? `${depth}-bit / ${formatSampleRate(rate)}`
      : rate
        ? formatSampleRate(rate)
        : null;
  const shortRate = rate
    ? rate % 1000 === 0
      ? String(rate / 1000)
      : (rate / 1000).toFixed(1)
    : null;
  return {
    name,
    resolution,
    badge: depth && shortRate ? `${depth}/${shortRate}` : name,
    channels,
    tier: audio.lossless === true ? (hiRes ? "hi_res" : "lossless") : "unknown",
  };
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
