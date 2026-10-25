import type { AudioProperties, Loudness } from "../domain/types";
import {
  activeEqBands,
  crossfeedActive,
  crossfeedParameters,
  crossfeedStrengthLabels,
  describeEqBand,
  dspMayClip,
  headroomActive,
  recommendedHeadroomDb,
  requestedContextSampleRate,
  type DspSettings,
} from "./dsp";

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

export type SignalStreamTarget = "direct" | "cast" | "native";

/** Live measurements of the server's DSP chain while it plays. */
export interface ServerDspStatus {
  /**
   * Seconds of audio processed per second of processing time (Roon's
   * "processing speed"); `null` until there is enough to measure.
   */
  realtime_factor: number | null;
  /** Highest level after the chain, before dither, in dBFS. */
  peak_dbfs: number | null;
  /** Samples that went past full scale and were limited. */
  clipped_samples: number;
  clipping: boolean;
}

/** The server's own DSP chain on a server output. */
export interface ServerSignalProcessing {
  /** e.g. `Parson DSP, 64-bit float`. */
  engine: string;
  /** Stage ids the chain runs, in order. */
  stages: string[];
  /** What the chain could not do as asked. */
  notes: string[];
  /** `null` when the path is only described, not playing. */
  status: ServerDspStatus | null;
}

/** Server leg returned by `GET /media/songs/{id}/signal-path`. */
export interface ServerSignalPath {
  song_id: string;
  target: SignalStreamTarget;
  mode: "original" | "transcode" | "native";
  quality: SignalQuality;
  source: SignalFormat;
  delivered: SignalFormat;
  content_type: string;
  requested_bitrate_kbps: number;
  effect: string | null;
  stages: SignalStage[];
  /** Present when a server output runs the DSP chain itself. */
  processing?: ServerSignalProcessing | null;
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
  /** The DSP chain in the Web Audio graph (`web-audio` engine only). */
  dsp?: DspSettings | null;
  /** The chain's crossfeed needs channel splitters this engine lacks. */
  crossfeedUnsupported?: boolean;
  /**
   * Rate the output device runs at, when the client can read it. A
   * difference from the stream's rate means the browser resamples.
   */
  deviceSampleRate?: number | null;
  output:
    | {
        kind: "system";
        /** Device label, e.g. from `enumerateDevices()`. */
        name?: string | null;
        /** The user picked this device rather than the system default. */
        chosen?: boolean;
      }
    | { kind: "cast"; name?: string | null }
    /**
     * A device attached to the server, played by the server itself. The
     * server leg then covers the whole path, so this client adds nothing.
     */
    | { kind: "native"; name?: string | null };
}

export interface SignalPath {
  quality: SignalQuality;
  /**
   * Lossless source above CD resolution, and the path is not lossy: a
   * high-resolution file transcoded to MP3 is not high resolution.
   */
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
  /**
   * The whole path is lossless including the output stage, which is only
   * possible on a server output opened in exclusive mode.
   */
  bitPerfect: boolean;
  /**
   * Processing ran on the server and its result reaches a server output
   * in exclusive mode with nothing after it: the DAC receives exactly
   * the samples Parson computed.
   */
  exactAfterProcessing: boolean;
  /** The server's DSP chain, when it runs one. */
  processing: ServerSignalProcessing | null;
  /** The chain measured samples past full scale: the light turns red. */
  clipping: boolean;
  /**
   * One sentence naming what decided the light, built from the stages
   * present: what you asked for, what you did not, what was lossy.
   */
  description: string;
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

/** The light's colour when the DSP chain clips. */
export const SIGNAL_CLIPPING_COLOR = "#ff4d5e";

/** Colour of a path's light: red while clipping, else its quality. */
export function signalLightColor(
  path: Pick<SignalPath, "quality" | "clipping">,
) {
  return path.clipping
    ? SIGNAL_CLIPPING_COLOR
    : signalQualityColors[path.quality];
}

/** Label of a path's light, e.g. `Enhanced` or `Enhanced, clipping`. */
export function signalLightLabel(
  path: Pick<SignalPath, "quality" | "clipping">,
) {
  const label = signalQualityLabels[path.quality];
  return path.clipping ? `${label}, clipping` : label;
}

/** `38.2x`, Roon's processing speed figure. */
export function formatRealtimeFactor(factor: number) {
  if (!Number.isFinite(factor) || factor <= 0) return "—";
  return factor >= 100 ? `${Math.round(factor)}x` : `${factor.toFixed(1)}x`;
}

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

function dspStages(
  dsp: DspSettings,
  crossfeedUnsupported: boolean,
): SignalStage[] {
  const stages: SignalStage[] = [];
  if (headroomActive(dsp)) {
    stages.push({
      id: "dsp-headroom",
      kind: "dsp",
      title: "Headroom adjustment",
      detail: formatDecibels(dsp.headroom.gainDb),
      quality: "enhanced",
      explanation:
        "Headroom management lowers the level before the EQ so its boosts cannot push loud passages past full scale and clip.",
    });
  }
  const bands = activeEqBands(dsp);
  if (bands.length) {
    stages.push({
      id: "dsp-eq",
      kind: "dsp",
      title: "Parametric EQ",
      detail: `${bands.length} ${bands.length === 1 ? "band" : "bands"}: ${bands.map(describeEqBand).join(", ")}`,
      quality: "enhanced",
      explanation: `A parametric EQ you set up, built from ${bands.length} biquad filter${bands.length === 1 ? "" : "s"} running in 32-bit float.${
        dspMayClip(dsp)
          ? ` Its largest boost needs about ${formatDecibels(recommendedHeadroomDb(dsp))} of headroom to avoid clipping; turn on headroom adjustment.`
          : ""
      }`,
    });
  }
  if (crossfeedActive(dsp)) {
    const params = crossfeedParameters[dsp.crossfeed.strength];
    stages.push({
      id: "dsp-crossfeed",
      kind: "dsp",
      title: "Crossfeed",
      detail: crossfeedUnsupported
        ? "Not available on this device"
        : `${crossfeedStrengthLabels[dsp.crossfeed.strength]}: ${params.cutoffHz} Hz, ${formatDecibels(params.feedDb)}, ${params.delayMs} ms`,
      quality: crossfeedUnsupported ? null : "enhanced",
      explanation: crossfeedUnsupported
        ? "This audio engine cannot split channels, so crossfeed is skipped."
        : "Crossfeed mixes a delayed, low-passed share of each channel into the other ear, so headphone listening sounds less like the music is inside your head.",
    });
  }
  return stages;
}

function clientStages(
  client: ClientSignalLeg,
  delivered: SignalFormat | null,
): SignalStage[] {
  const stages: SignalStage[] = [];
  if (client.output.kind === "native") return stages;
  const casting = client.output.kind === "cast";
  const deviceName = casting
    ? client.output.name || "Cast device"
    : PLATFORM_DEVICE[client.platform];
  const codec = delivered?.codec
    ? (CODEC_LABELS[delivered.codec] ?? delivered.codec.toUpperCase())
    : "The stream";
  const webAudio = !casting && client.engine === "web-audio";
  const dsp = webAudio && client.dsp?.enabled ? client.dsp : null;

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
          ? `${codec} is decoded and passes through a Web Audio graph so processing can be applied. Decoding itself is exact.`
          : `${codec} is decoded by the platform's own decoder and handed straight to the output. Decoding itself is exact.`,
    });
  }

  if (webAudio) {
    const depth = delivered?.lossless ? delivered.bit_depth : null;
    stages.push({
      id: "client-depth-in",
      kind: "conversion",
      title: "Bit depth conversion",
      detail: depth ? `${depth}bit to 32bit float` : "Decoded to 32bit float",
      quality: depth && depth > 24 ? "high_quality" : "lossless",
      explanation:
        depth && depth > 24
          ? `32-bit float keeps 24 bits of precision, so the lowest ${depth - 24} bits of this ${depth}-bit file are rounded.`
          : "32-bit float holds every 16- and 24-bit sample exactly, so this conversion loses nothing.",
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

  if (!casting && client.leveling) {
    const leveling = client.leveling;
    const limited =
      leveling.limitedBy === "peak"
        ? ` (limited from ${formatDecibels(leveling.requestedDb)} to avoid clipping)`
        : leveling.limitedBy === "full_volume"
          ? ` (${formatDecibels(leveling.requestedDb)} requested; lower the volume to leave room for it)`
          : "";
    const unchanged =
      Math.abs(leveling.appliedDb) < 0.05 && leveling.limitedBy === null;
    stages.push({
      id: "client-leveling",
      kind: "volume",
      title: "Volume leveling",
      detail: `${formatDecibels(leveling.appliedDb)}, ${leveling.source} ReplayGain${limited}`,
      quality: unchanged ? "lossless" : "enhanced",
      explanation:
        "Volume leveling, which you turned on, adjusts each track to the same loudness using its ReplayGain tags, so tracks from different albums play at a similar volume.",
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
          "Parson's volume slider scales the samples digitally, like Roon's DSP volume. At full volume this stage disappears; use your system or amplifier volume for a bit-perfect path.",
      });
    }
  }

  const streamRate = delivered?.sample_rate ?? null;
  const graphRate = webAudio ? (client.contextSampleRate ?? null) : null;
  if (graphRate && streamRate && graphRate !== streamRate) {
    const chosen =
      requestedContextSampleRate(client.dsp, streamRate) === graphRate;
    stages.push({
      id: "client-resample",
      kind: "conversion",
      title: "Sample rate conversion",
      detail: `${formatSampleRate(streamRate)} to ${formatSampleRate(graphRate)}`,
      quality: chosen ? "enhanced" : "high_quality",
      explanation: chosen
        ? "You chose this processing rate in the DSP engine, so the stream is resampled to it."
        : "The audio graph runs at the output device's rate, so the stream is resampled to match. High quality, but not bit-perfect. Choose 'Match source' in the DSP engine's sample rate section to avoid it.",
    });
  }

  if (dsp) stages.push(...dspStages(dsp, Boolean(client.crossfeedUnsupported)));

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

  if (webAudio) {
    stages.push({
      id: "client-depth-out",
      kind: "conversion",
      title: "Bit depth conversion",
      detail: "32bit float to output format",
      quality: "lossless",
      explanation:
        "The graph's 32-bit float output is handed to the system in its own format. Samples that were not processed convert back exactly.",
    });
  }

  const outputRate = graphRate ?? streamRate;
  if (
    !casting &&
    client.deviceSampleRate &&
    outputRate &&
    client.deviceSampleRate !== outputRate
  ) {
    stages.push({
      id: "client-device-resample",
      kind: "conversion",
      title: "Sample rate conversion",
      detail: `${formatSampleRate(outputRate)} to ${formatSampleRate(client.deviceSampleRate)}`,
      quality: "high_quality",
      explanation: `The output device runs at ${formatSampleRate(client.deviceSampleRate)}, so the ${client.platform === "desktop" ? "app" : "browser"} resamples the stream for it. Set the device to ${formatSampleRate(outputRate)} in your system's sound settings for a bit-perfect handoff.`,
    });
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
    const output = client.output.kind === "system" ? client.output : null;
    const mixer =
      client.platform === "ios"
        ? "iOS audio session"
        : client.platform === "android"
          ? "Android audio mixer"
          : output?.chosen
            ? "Selected output, shared mode"
            : "System default output, shared mode";
    const name =
      output?.name ||
      (client.platform === "ios" || client.platform === "android"
        ? "System output"
        : "System default output");
    stages.push({
      id: "client-output",
      kind: "output",
      title: name,
      detail: client.deviceSampleRate
        ? `${mixer}, ${formatSampleRate(client.deviceSampleRate)}`
        : mixer,
      // Roon rates an OS mixer output "High quality": in shared mode the
      // system may resample, apply its volume and mix in other sounds, and
      // nothing tells Parson which formats the device itself accepts.
      quality: "high_quality",
      explanation: `${name} is reached through the ${
        client.platform === "ios"
          ? "iOS audio session"
          : client.platform === "android"
            ? "Android audio mixer"
            : "system mixer in shared mode"
      }, so the system can resample the audio, apply its volume and mix in other sounds after Parson hands it over. ${
        client.platform === "web" || client.platform === "desktop"
          ? "For a bit-perfect path, play on a server output in exclusive mode (Settings → Audio outputs)."
          : "Phones always play through the system mixer; play on a server output for a bit-perfect path."
      }`,
    });
  }
  return stages;
}

/**
 * Every classified stage after the first lossy one carries lossy audio, so
 * it is rated low quality too: an exact copy of an MP3 is still an MP3.
 */
export function carryLossyGeneration(stages: SignalStage[]): SignalStage[] {
  const first = stages.findIndex((stage) => stage.quality === "low_quality");
  if (first < 0) return stages;
  return stages.map((stage, index) =>
    index > first && stage.quality
      ? { ...stage, quality: "low_quality" }
      : stage,
  );
}

function listTitles(stages: SignalStage[]) {
  const titles = [
    ...new Set(
      stages.map((stage) =>
        stage.kind === "dsp" || stage.kind === "volume"
          ? `${stage.title} (${stage.detail})`
          : stage.kind === "conversion"
            ? `${stage.title.toLowerCase()} ${stage.detail}`
            : stage.title.toLowerCase(),
      ),
    ),
  ];
  if (titles.length <= 1) return titles.join("");
  return `${titles.slice(0, -1).join(", ")} and ${titles.at(-1)}`;
}

/**
 * The sentence under the light. It names the stages that decided it, so a
 * path you processed on purpose never reads as an accident.
 */
export function describeSignalPath(
  stages: SignalStage[],
  options: { highRes?: boolean; output?: string } = {},
): string {
  const quality = overallSignalQuality(stages);
  const by = (wanted: SignalQuality) =>
    stages.filter((stage) => stage.quality === wanted);
  const enhanced = by("enhanced");
  const outputStage = stages.find((stage) => stage.kind === "output");
  const sharedOutput =
    outputStage?.quality === "high_quality" ? outputStage : null;
  const high = by("high_quality").filter((stage) => stage !== sharedOutput);
  const output = options.output ?? "the output";
  const asked = enhanced.length
    ? `processing you asked for: ${listTitles(enhanced)}`
    : "";
  switch (quality) {
    case "lossless":
      return outputStage?.id === "native-output"
        ? `${options.highRes ? "A high-resolution lossless" : "A lossless"} stream reaches ${output} bit-perfect, in exclusive mode at ${outputStage.detail.split(", ").at(-1)}.`
        : `${options.highRes ? "A high-resolution lossless" : "A lossless"} stream reaches ${output} without being modified.`;
    case "enhanced":
      return outputStage?.id === "native-output" &&
        outputStage.quality === "lossless"
        ? `Parson's server is processing the audio because you asked it to: ${listTitles(enhanced.filter((stage) => stage.kind !== "conversion" || stage.id === "native-resample"))}. The result reaches ${output} bit-exact, in exclusive mode at ${outputStage.detail.split(", ").at(-1)}.`
        : `Parson is processing the audio because you asked it to: ${listTitles(enhanced)}. Nothing else changes it.`;
    case "high_quality": {
      const unknown = stages.some(
        (stage) => stage.kind === "source" && !stage.quality,
      );
      const mixer = sharedOutput
        ? `${output} is reached in shared mode, where the system can resample the audio and mix in other sounds, so Parson cannot promise it arrives unmodified.`
        : "";
      const changed = high.length
        ? `The audio is changed in ways you did not ask for, with very little loss: ${listTitles(high)}.`
        : unknown
          ? "Parson cannot tell whether the source is lossless, so the path is rated high quality at best."
          : mixer
            ? ""
            : "The path is rated high quality.";
      const sentence = [changed, mixer].filter(Boolean).join(" ");
      return asked ? `${sentence} It also applies ${asked}.` : sentence;
    }
    case "low_quality": {
      const lossy = stages.find((stage) => stage.quality === "low_quality");
      const what = lossy
        ? lossy.kind === "source"
          ? `the source is a lossy file (${lossy.detail})`
          : `${lossy.title.toLowerCase()} to ${lossy.detail}`
        : "a lossy format";
      return `A lossy format is involved: ${what}.${asked ? ` Parson also applies ${asked}.` : ""}`;
    }
  }
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
  const rated = carryLossyGeneration(stages);
  const quality = overallSignalQuality(rated);
  // A transcoded MP3 is not hi-res, whatever the file it came from.
  const highRes = Boolean(source?.high_res) && quality !== "low_quality";
  const outputStage = stages.find((stage) => stage.kind === "output");
  const processing =
    server && !client.localFile ? (server.processing ?? null) : null;
  const clipping = processing?.status?.clipping === true;
  const clipped = processing?.status?.clipped_samples ?? 0;
  const description = `${describeSignalPath(stages, {
    highRes,
    output: outputStage?.title,
  })}${
    clipping
      ? ` Clipping detected: ${clipped} ${clipped === 1 ? "sample" : "samples"} went past full scale and were limited. Lower the headroom adjustment in the DSP engine.`
      : ""
  }`;
  const beforeOutput = rated.filter((stage) => stage.kind !== "output");
  const ratedOutput = rated.find((stage) => stage.kind === "output");
  return {
    exactAfterProcessing:
      quality === "enhanced" &&
      Boolean(processing) &&
      ratedOutput?.id === "native-output" &&
      ratedOutput.quality === "lossless",
    processing,
    clipping,
    bitPerfectToOutput:
      beforeOutput.length > 0 &&
      beforeOutput.every((stage) => stage.quality === "lossless"),
    bitPerfect: quality === "lossless" && outputStage?.id === "native-output",
    quality,
    highRes,
    source,
    delivered,
    stages: rated,
    serverKnown: Boolean(server) || Boolean(client.localFile),
    description,
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
