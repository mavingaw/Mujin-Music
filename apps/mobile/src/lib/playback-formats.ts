import type {
  AudioProperties,
  PlaybackCapabilities,
  PlaybackFormatToken,
} from "@parson/music-sdk";

/**
 * Formats the native decoders play on each platform. Anything else (DSD,
 * WavPack, APE, TAK, WMA, ALAC on Android, ...) is converted losslessly by
 * the server, so every track in the library plays.
 */
const NATIVE_FORMATS: Record<"android" | "ios", PlaybackFormatToken[]> = {
  android: ["flac", "mp3", "aac", "vorbis", "opus", "wav"],
  ios: ["flac", "alac", "mp3", "aac", "wav", "aiff"],
};

/** Phones resample internally; higher sources are fitted within their rate family. */
export const MOBILE_MAX_SAMPLE_RATE = 192_000;

export function nativePlaybackCapabilities(os: string): PlaybackCapabilities {
  const accept = NATIVE_FORMATS[os === "ios" ? "ios" : "android"];
  return { accept: [...accept], maxSampleRate: MOBILE_MAX_SAMPLE_RATE };
}

const EXTENSION_TOKENS: Record<string, PlaybackFormatToken> = {
  flac: "flac",
  mp3: "mp3",
  aac: "aac",
  ogg: "vorbis",
  oga: "vorbis",
  opus: "opus",
  wav: "wav",
  aiff: "aiff",
  aif: "aiff",
  alac: "alac",
};

/** The token a device must accept to play the original file as-is. */
export function sourceFormatToken(
  path: string,
  audio?: AudioProperties | null,
): PlaybackFormatToken | null {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  const codec = audio?.codec ?? null;
  if (extension === "m4a" || extension === "mp4") {
    if (codec === "alac") return "alac";
    if (codec === "aac") return "aac";
    return null;
  }
  if (codec && codec !== "pcm" && extension === "ogg") {
    return codec === "opus" ? "opus" : codec === "vorbis" ? "vorbis" : null;
  }
  if (codec === "pcm_float") return null;
  return EXTENSION_TOKENS[extension] ?? null;
}

/** Whether the original file plays on a device without conversion. */
export function playsNatively(
  path: string,
  audio: AudioProperties | null | undefined,
  capabilities: PlaybackCapabilities,
) {
  const token = sourceFormatToken(path, audio);
  if (!token || !capabilities.accept.includes(token)) return false;
  const rate = audio?.sample_rate ?? 0;
  return !capabilities.maxSampleRate || rate <= capabilities.maxSampleRate;
}

/**
 * Path whose extension names what an offline copy will contain: the original
 * when it plays natively, otherwise the server's lossless FLAC conversion.
 */
export function offlineMediaPath(
  path: string,
  audio: AudioProperties | null | undefined,
  capabilities: PlaybackCapabilities,
) {
  return playsNatively(path, audio, capabilities) ? path : `${path}.flac`;
}
