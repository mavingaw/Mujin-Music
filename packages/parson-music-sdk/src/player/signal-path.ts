import type { AudioProperties, Loudness } from "../domain/types";

/**
 * Roon-style quality classes, best first:
 * - `lossless` (purple): the bits reach the output unmodified.
 * - `enhanced` (blue): processing you asked for (DSP, speed, leveling).
 * - `high_quality` (green): high-fidelity changes you did not ask for
 *   (compatibility resampling, software volume, an OS mixer).
 * - `low_quality` (yellow): a lossy format is involved somewhere.
 */
export type SignalQuality =
  "lossless" | "enhanced" | "high_quality" | "low_quality";

export type SignalStageKind =
  | "source"
  | "dsp"
  | "conversion"
  | "encode"
  | "transport"
  | "decode"
  | "volume"
  | "output";

export interface SignalStage {
  /** Stable key, unique within one path. */
  id: string;
  kind: SignalStageKind;
  title: string;
  detail: string;
  /** `null` when the stage cannot be classified (unknown codec). */
  quality: SignalQuality | null;
  /** Plain-language explanation shown when the stage is selected. */
  explanation: string;
}

export interface SignalFormat {
  codec: string | null;
  container: string | null;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
  bitrate_kbps: number | null;
  lossless: boolean | null;
  /** Lossless above CD resolution (more than 16 bit or 48 kHz). */
  high_res: boolean;
  /** e.g. `FLAC 96kHz 24bit 2ch`. */
  summary: string;
}

export type SignalStreamTarget = "direct" | "cast";

/** Server leg returned by `GET /media/songs/{id}/signal-path`. */
export interface ServerSignalPath {
  song_id: string;
  target: SignalStreamTarget;
  mode: "original" | "transcode";
  quality: SignalQuality;
  source: SignalFormat;
  delivered: SignalFormat;
  content_type: string;
  requested_bitrate_kbps: number;
  effect: string | null;
  stages: SignalStage[];
}

export type SignalPlatform = "web" | "desktop" | "ios" | "android";

export interface VolumeLevelingResult {
  mode: VolumeLevelingMode;
  /** Gain the tags ask for, after the preamp. */
  requestedDb: number;
  /** Gain actually applied. */
  appliedDb: number;
  /** Which tag the gain came from. */
  source: "track" | "album";
  /** Why the applied gain is lower than requested, if it is. */
  limitedBy: "peak" | "full_volume" | null;
}

export type VolumeLevelingMode = "off" | "track" | "album" | "auto";

/** What the playing client does with the stream. */
export interface ClientSignalLeg {
  platform: SignalPlatform;
  /** `true` when a downloaded copy plays instead of a server stream. */
  localFile?: boolean;
  /** `web-audio` when samples pass through an AudioContext graph. */
  engine: "media-element" | "web-audio" | "native";
  /** `AudioContext.sampleRate` for the `web-audio` engine. */
  contextSampleRate?: number | null;
  playbackRate: number;
  preservePitch: boolean;
  /** Convolution reverb mix, 0..1. */
  reverbWet: number;
  presetLabel?: string | null;
  /** Software volume 0..1; omit on clients that use the system volume. */
  volume?: number | null;
  muted?: boolean;
  leveling?: VolumeLevelingResult | null;
  /** Leveling is on but this track has no ReplayGain tags. */
  levelingUnavailable?: boolean;
  output:
    | { kind: "system"; name?: string | null }
    | { kind: "cast"; name?: string | null };
}

export interface SignalPath {
  quality: SignalQuality;
  /** Lossless source above CD resolution. */
  highRes: boolean;
  /** Source format, when known. */
  source: SignalFormat | null;
  /** Format arriving at the client, when known. */
  delivered: SignalFormat | null;
  stages: SignalStage[];
  /** `false` when the server leg could not be loaded. */
  serverKnown: boolean;
  /**
   * Every stage before the output is lossless: the samples reach the
   * output (OS mixer or Cast device) exactly as they are in the file.
   */
  bitPerfectToOutput: boolean;
}

const QUALITY_RANK: Record<SignalQuality, number> = {
  low_quality: 0,
  high_quality: 1,
  enhanced: 2,
  lossless: 3,
};

export const signalQualityLabels: Record<SignalQuality, string> = {
  lossless: "Lossless",
  enhanced: "Enhanced",
  high_quality: "High quality",
  low_quality: "Low quality",
};

/** Light colours, matching Roon's purple / blue / green / yellow. */
export const signalQualityColors: Record<SignalQuality, string> = {
  lossless: "#b26bff",
  enhanced: "#38b6ff",
  high_quality: "#3ddc97",
  low_quality: "#f5c542",
};

export const signalQualityDescriptions: Record<SignalQuality, string> = {
  lossless:
    "The audio reaches the output without being modified, as far as Parson can see.",
  enhanced:
    "Parson is processing the audio because you asked it to, for example volume leveling or a sound preset.",
  high_quality:
    "The audio is changed in ways you did not ask for, but with very little loss: resampling for compatibility, software volume or the system mixer.",
  low_quality: "A lossy format is involved somewhere in the path.",
};

export function worseQuality(
  left: SignalQuality,
  right: SignalQuality,
): SignalQuality {
  return QUALITY_RANK[left] <= QUALITY_RANK[right] ? left : right;
}

/** Worst classified stage; an unclassified source caps the path at high quality. */
export function overallSignalQuality(stages: SignalStage[]): SignalQuality {
  let worst: SignalQuality | null = null;
  for (const stage of stages) {
    if (!stage.quality) continue;
    worst = worst ? worseQuality(worst, stage.quality) : stage.quality;
  }
  const result = worst ?? "high_quality";
  return stages.some((stage) => stage.kind === "source" && !stage.quality)
    ? worseQuality(result, "high_quality")
    : result;
}

/** `44.1kHz`, `96kHz`. */
export function formatSampleRate(rate: number) {
  const khz = rate / 1000;
  return Number.isInteger(khz) ? `${khz}kHz` : `${khz.toFixed(1)}kHz`;
}

export function formatDecibels(db: number) {
  const rounded = Math.round(db * 10) / 10;
  if (rounded === 0) return "0.0 dB";
  return `${rounded > 0 ? "+" : "−"}${Math.abs(rounded).toFixed(1)} dB`;
}

const CODEC_LABELS: Record<string, string> = {
  flac: "FLAC",
  alac: "ALAC",
  aac: "AAC",
  mp3: "MP3",
  vorbis: "Ogg Vorbis",
  opus: "Opus",
  pcm: "PCM",
  pcm_float: "PCM float",
  wavpack: "WavPack",
  ape: "Monkey's Audio",
};

/** Short format label for songs without a server leg (badges, offline). */
export function describeAudioFormat(
  audio: AudioProperties | null | undefined,
): SignalFormat | null {
  if (!audio) return null;
  const codec = audio.codec;
  const parts: string[] = [];
  if (codec) parts.push(CODEC_LABELS[codec] ?? codec.toUpperCase());
  else if (audio.container) parts.push(audio.container.toUpperCase());
  else parts.push("Unknown format");
  if (audio.sample_rate) parts.push(formatSampleRate(audio.sample_rate));
  if (audio.bit_depth) parts.push(`${audio.bit_depth}bit`);
  if (audio.channels) parts.push(`${audio.channels}ch`);
  if (audio.lossless !== true && audio.bitrate_kbps)
    parts.push(`${audio.bitrate_kbps}kbps`);
  const highRes =
    audio.lossless === true &&
    ((audio.sample_rate ?? 0) > 48_000 || (audio.bit_depth ?? 0) > 16);
  return {
    codec: audio.codec,
    container: audio.container,
    sample_rate: audio.sample_rate,
    bit_depth: audio.bit_depth,
    channels: audio.channels,
    bitrate_kbps: audio.bitrate_kbps,
    lossless: audio.lossless,
    high_res: highRes,
    summary: parts.join(" "),
  };
}

/** Quality of a file on its own, for format badges. */
export function sourceQuality(
  audio: AudioProperties | null | undefined,
): SignalQuality | null {
  if (!audio || audio.lossless === null || audio.lossless === undefined)
    return null;
  return audio.lossless ? "lossless" : "low_quality";
}

/**
 * ReplayGain gain for a track. `auto` uses the album gain when the queue
 * plays an album in order and the track gain otherwise (the caller decides
 * which by passing `albumContext`). Positive gains are limited so the
 * track peak does not clip, and by `maxGainDb` (clients that can only
 * attenuate pass the headroom their volume control leaves).
 */
export function volumeLevelingGain(
  loudness: Loudness | null | undefined,
  mode: VolumeLevelingMode,
  options: {
    preampDb?: number;
    albumContext?: boolean;
    maxGainDb?: number;
  } = {},
): VolumeLevelingResult | null {
  if (!loudness || mode === "off") return null;
  const preferAlbum =
    mode === "album" || (mode === "auto" && options.albumContext === true);
  const albumGain = loudness.album_gain_db ?? null;
  const trackGain = loudness.track_gain_db ?? null;
  const useAlbum = preferAlbum ? albumGain !== null : trackGain === null;
  const gain = useAlbum ? albumGain : trackGain;
  if (gain === null) return null;
  const peak = useAlbum
    ? (loudness.album_peak ?? loudness.track_peak ?? null)
    : (loudness.track_peak ?? null);
  const requestedDb = gain + (options.preampDb ?? 0);
  let appliedDb = requestedDb;
  let limitedBy: VolumeLevelingResult["limitedBy"] = null;
  if (peak && peak > 0) {
    const headroom = -20 * Math.log10(peak);
    if (appliedDb > headroom) {
      appliedDb = headroom;
      limitedBy = "peak";
    }
  }
  if (options.maxGainDb !== undefined && appliedDb > options.maxGainDb) {
    appliedDb = options.maxGainDb;
    limitedBy = "full_volume";
  }
  return {
    mode,
    requestedDb,
    appliedDb,
    source: useAlbum ? "album" : "track",
    limitedBy,
  };
}

export function decibelsToGain(db: number) {
  return 10 ** (db / 20);
}

export function gainToDecibels(gain: number) {
  return 20 * Math.log10(gain);
}

const PLATFORM_DEVICE: Record<SignalPlatform, string> = {
  web: "This browser",
  desktop: "Parson desktop",
  ios: "This iPhone",
  android: "This Android device",
};

function sourceStageFromAudio(
  format: SignalFormat | null,
  localFile: boolean,
): SignalStage {
  const quality: SignalQuality | null =
    format?.lossless === true
      ? "lossless"
      : format?.lossless === false
        ? "low_quality"
        : null;
  return {
    id: "source",
    kind: "source",
    title: localFile ? "Source (downloaded)" : "Source",
    detail: format?.summary ?? "Unknown format",
    quality,
    explanation:
      quality === "lossless"
        ? "A lossless file: decoding it reproduces the original samples exactly."
        : quality === "low_quality"
          ? "A lossy file. Detail was discarded when it was encoded, so the path can be no better than low quality."
          : "Parson could not identify this file's codec, so it cannot say whether the source is lossless.",
  };
}

function clientStages(
  client: ClientSignalLeg,
  delivered: SignalFormat | null,
): SignalStage[] {
  const stages: SignalStage[] = [];
  const casting = client.output.kind === "cast";
  const deviceName = casting
    ? client.output.name || "Cast device"
    : PLATFORM_DEVICE[client.platform];
  const codec = delivered?.codec
    ? (CODEC_LABELS[delivered.codec] ?? delivered.codec.toUpperCase())
    : "The stream";

  if (casting) {
    stages.push({
      id: "client-decode",
      kind: "decode",
      title: deviceName,
      detail: "Google Cast receiver",
      quality: "lossless",
      explanation: `${codec} is fetched and decoded by the receiver itself. Parson only sends commands; the audio does not pass through this device.`,
    });
  } else {
    const engineDetail =
      client.engine === "web-audio"
        ? client.contextSampleRate
          ? `Web Audio graph at ${formatSampleRate(client.contextSampleRate)}`
          : "Web Audio graph"
        : client.engine === "native"
          ? client.platform === "android"
            ? "Media3 ExoPlayer"
            : "Native player"
          : "HTML audio element";
    stages.push({
      id: "client-decode",
      kind: "decode",
      title: deviceName,
      detail: engineDetail,
      quality: "lossless",
      explanation:
        client.engine === "web-audio"
          ? `${codec} is decoded to 32-bit float and passes through a Web Audio graph so effects can be applied. Decoding itself is exact.`
          : `${codec} is decoded by the platform's own decoder. Decoding itself is exact.`,
    });
  }

  if (
    !casting &&
    client.engine === "web-audio" &&
    client.contextSampleRate &&
    delivered?.sample_rate &&
    client.contextSampleRate !== delivered.sample_rate
  ) {
    stages.push({
      id: "client-resample",
      kind: "conversion",
      title: "Sample rate conversion",
      detail: `${formatSampleRate(delivered.sample_rate)} to ${formatSampleRate(client.contextSampleRate)}`,
      quality: "high_quality",
      explanation:
        "The audio graph runs at the output device's rate, so the stream is resampled to match. High quality, but not bit-perfect.",
    });
  }

  if (!casting && Math.abs(client.playbackRate - 1) > 1e-6) {
    stages.push({
      id: "client-speed",
      kind: "dsp",
      title: client.presetLabel
        ? `${client.presetLabel}: speed`
        : "Playback speed",
      detail: `${client.playbackRate.toFixed(2)}x, ${client.preservePitch ? "pitch kept" : "pitch follows speed"}`,
      quality: "enhanced",
      explanation: client.preservePitch
        ? "The sound preset you chose time-stretches the audio to change its speed without changing pitch."
        : "The sound preset you chose resamples the audio, so speed and pitch change together, like a turntable.",
    });
  }

  if (!casting && client.reverbWet > 0) {
    stages.push({
      id: "client-reverb",
      kind: "dsp",
      title: client.presetLabel ? `${client.presetLabel}: reverb` : "Reverb",
      detail: `Convolution, 1.8s room, ${Math.round(client.reverbWet * 100)}% wet`,
      quality: "enhanced",
      explanation:
        "A convolution reverb mixed in by the sound preset you chose.",
    });
  }

  if (!casting && client.leveling) {
    const leveling = client.leveling;
    const limited =
      leveling.limitedBy === "peak"
        ? ` (limited from ${formatDecibels(leveling.requestedDb)} to avoid clipping)`
        : leveling.limitedBy === "full_volume"
          ? ` (${formatDecibels(leveling.requestedDb)} requested; lower the volume to leave room for it)`
          : "";
    stages.push({
      id: "client-leveling",
      kind: "volume",
      title: "Volume leveling",
      detail: `${formatDecibels(leveling.appliedDb)}, ${leveling.source} ReplayGain${limited}`,
      quality:
        Math.abs(leveling.appliedDb) < 0.05 && leveling.limitedBy === null
          ? "lossless"
          : "enhanced",
      explanation:
        "Volume leveling adjusts each track to the same loudness using its ReplayGain tags, so tracks from different albums play at a similar volume.",
    });
  } else if (!casting && client.levelingUnavailable) {
    stages.push({
      id: "client-leveling",
      kind: "volume",
      title: "Volume leveling",
      detail: "No ReplayGain tags on this track",
      quality: null,
      explanation:
        "Volume leveling is on, but this file has no ReplayGain or R128 tags, so it plays unchanged. Tag your library with a ReplayGain scanner to level it.",
    });
  }

  if (!casting && client.volume !== undefined && client.volume !== null) {
    if (client.muted || client.volume <= 0) {
      stages.push({
        id: "client-volume",
        kind: "volume",
        title: "Volume",
        detail: "Muted",
        quality: "high_quality",
        explanation: "Playback is muted in Parson.",
      });
    } else if (client.volume < 0.9995) {
      stages.push({
        id: "client-volume",
        kind: "volume",
        title: "Software volume",
        detail: formatDecibels(gainToDecibels(client.volume)),
        quality: "high_quality",
        explanation:
          "Parson's volume slider scales the samples digitally. At full volume this stage disappears; use your system or amplifier volume for a bit-perfect path.",
      });
    }
  }

  if (casting) {
    stages.push({
      id: "client-output",
      kind: "output",
      title: "Output",
      detail: `${deviceName}, receiver volume`,
      quality: "lossless",
      explanation:
        "The Cast device plays the stream and applies its own volume. Parson cannot see what the device does after that.",
    });
  } else {
    const mixer =
      client.platform === "ios"
        ? "iOS audio session"
        : client.platform === "android"
          ? "Android audio mixer"
          : "System mixer";
    stages.push({
      id: "client-output",
      kind: "output",
      title: client.output.name || "System output",
      detail: mixer,
      quality: "high_quality",
      explanation:
        "The audio is handed to the operating system mixer, which may resample, mix other sounds in or apply its own volume. Parson cannot guarantee that this last step is bit-perfect, so it is labelled high quality, as Roon does for OS mixer outputs.",
    });
  }
  return stages;
}

/** Combines the server leg (when known) with this client's leg. */
export function buildSignalPath(
  server: ServerSignalPath | null,
  client: ClientSignalLeg,
  fallbackAudio?: AudioProperties | null,
): SignalPath {
  const fallbackFormat = describeAudioFormat(fallbackAudio);
  const stages: SignalStage[] = [];
  let source: SignalFormat | null;
  let delivered: SignalFormat | null;
  if (server && !client.localFile) {
    stages.push(...server.stages);
    source = server.source;
    delivered = server.delivered;
  } else {
    source = fallbackFormat;
    delivered = fallbackFormat;
    stages.push(
      sourceStageFromAudio(fallbackFormat, Boolean(client.localFile)),
    );
    if (client.localFile) {
      stages.push({
        id: "local-file",
        kind: "transport",
        title: "Offline copy",
        detail: "Original file stored on this device",
        quality: "lossless",
        explanation:
          "The downloaded copy is the original file, so nothing is converted before playback.",
      });
    }
  }
  stages.push(...clientStages(client, delivered));
  const beforeOutput = stages.filter((stage) => stage.kind !== "output");
  return {
    bitPerfectToOutput:
      beforeOutput.length > 0 &&
      beforeOutput.every((stage) => stage.quality === "lossless"),
    quality: overallSignalQuality(stages),
    highRes: Boolean(source?.high_res),
    source,
    delivered,
    stages,
    serverKnown: Boolean(server) || Boolean(client.localFile),
  };
}

/** One-line summary for headers, e.g. `FLAC 96kHz 24bit 2ch to MP3 320kbps`. */
export function signalPathSummary(path: SignalPath) {
  const source = path.source?.summary;
  const delivered = path.delivered?.summary;
  if (!source) return "Format unknown";
  if (!delivered || delivered === source) return source;
  return `${source} → ${delivered}`;
}

export interface CollectionFormat {
  /** e.g. `FLAC 44.1kHz 24bit`, `FLAC 44.1–96kHz 16–24bit`, `FLAC / MP3`. */
  summary: string;
  /** Worst source quality across the tracks, `null` when unknown. */
  quality: SignalQuality | null;
  /** Any track is high resolution. */
  highRes: boolean;
}

function range(values: number[], format: (value: number) => string) {
  if (!values.length) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  return min === max
    ? format(min)
    : `${format(min).replace(/[a-zA-Z]+$/, "")}–${format(max)}`;
}

/** Format line for an album or playlist, like Roon's album "Format" row. */
export function describeCollectionFormat(
  audios: readonly (AudioProperties | null | undefined)[],
): CollectionFormat | null {
  const known = audios.filter((audio): audio is AudioProperties =>
    Boolean(audio?.codec),
  );
  if (!known.length) return null;
  const codecs = [...new Set(known.map((audio) => audio.codec!))].map(
    (codec) => CODEC_LABELS[codec] ?? codec.toUpperCase(),
  );
  const parts = [codecs.slice(0, 3).join(" / ")];
  const rate = range(
    known.flatMap((audio) => (audio.sample_rate ? [audio.sample_rate] : [])),
    formatSampleRate,
  );
  if (rate) parts.push(rate);
  const depth = range(
    known.flatMap((audio) => (audio.bit_depth ? [audio.bit_depth] : [])),
    (value) => `${value}bit`,
  );
  if (depth) parts.push(depth);
  let quality: SignalQuality | null = null;
  for (const audio of audios) {
    const next = sourceQuality(audio);
    if (!next) {
      quality = null;
      break;
    }
    quality = quality ? worseQuality(quality, next) : next;
  }
  return {
    summary: parts.join(" "),
    quality,
    highRes: known.some(
      (audio) => describeAudioFormat(audio)?.high_res === true,
    ),
  };
}
