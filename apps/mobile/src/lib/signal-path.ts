import {
  defaultDspSettings,
  normalizeDspSettings,
  decibelsToGain,
  type DspSettings,
  volumeLevelingGain,
  type ClientSignalLeg,
  type Loudness,
  type PlayerAudioPreset,
  type SignalPlatform,
  type VolumeLevelingMode,
  type VolumeLevelingResult,
} from "@parson/music-sdk";

export const VOLUME_LEVELING_STORAGE_KEY = "parson.volume-leveling";
export const DSP_STORAGE_KEY = "parson.dsp";

/** Stored DSP settings, or the bypassed defaults. */
export function parseStoredDsp(value: string | null | undefined): DspSettings {
  if (!value) return defaultDspSettings();
  try {
    return normalizeDspSettings(JSON.parse(value));
  } catch {
    return defaultDspSettings();
  }
}

/** The effects graph that runs the DSP chain exists on iOS and web only. */
export function dspAvailable(platform: SignalPlatform) {
  return platform !== "android";
}

export const volumeLevelingOptions: readonly {
  mode: VolumeLevelingMode;
  label: string;
}[] = [
  { mode: "off", label: "Off" },
  { mode: "auto", label: "Auto" },
  { mode: "track", label: "Track" },
  { mode: "album", label: "Album" },
];

export function parseVolumeLevelingMode(
  value: string | null | undefined,
): VolumeLevelingMode {
  return volumeLevelingOptions.some((option) => option.mode === value)
    ? (value as VolumeLevelingMode)
    : "off";
}

export function signalPlatform(os: string): SignalPlatform {
  return os === "ios" ? "ios" : os === "android" ? "android" : "web";
}

/** True when a neighbour in the queue comes from the same album. */
export function playsInAlbumContext(
  albumIds: readonly (string | undefined)[],
  index: number,
) {
  const current = albumIds[index];
  if (!current) return false;
  return albumIds[index - 1] === current || albumIds[index + 1] === current;
}

export type NativeLeveling = {
  result: VolumeLevelingResult | null;
  /** Linear gain for the player volume or the effects graph. */
  gain: number;
  unavailable: boolean;
};

/**
 * Android's player volume cannot exceed 1, so boosts are limited there;
 * the iOS and web effects graph applies the gain with a GainNode.
 */
export function nativeLeveling(
  loudness: Loudness | null | undefined,
  mode: VolumeLevelingMode,
  platform: SignalPlatform,
  albumContext: boolean,
): NativeLeveling {
  if (mode === "off") return { result: null, gain: 1, unavailable: false };
  const result = volumeLevelingGain(loudness, mode, {
    albumContext,
    maxGainDb: platform === "android" ? 0 : undefined,
  });
  if (!result) return { result: null, gain: 1, unavailable: true };
  return { result, gain: decibelsToGain(result.appliedDb), unavailable: false };
}

export type MobileSignalState = {
  platform: SignalPlatform;
  /** A downloaded copy is playing. */
  localFile: boolean;
  /** Bitrate of the requested stream, 0 for the original file. */
  streamBitrate: number;
  /** `AudioContext.sampleRate` on iOS and web. */
  contextSampleRate: number | null;
  leveling: VolumeLevelingResult | null;
  levelingUnavailable: boolean;
  casting: boolean;
  castDeviceName: string | null;
  /** DSP chain in the effects graph; `null` where there is no graph. */
  dsp: DspSettings | null;
};

/** This phone's leg of the signal path. */
export function mobileClientLeg(
  state: MobileSignalState,
  preset: Pick<
    PlayerAudioPreset,
    "id" | "label" | "rate" | "wet" | "preservePitch"
  >,
): ClientSignalLeg {
  const processed = preset.id !== "original";
  return {
    platform: state.platform,
    localFile: state.localFile && !state.casting,
    engine: state.platform === "android" ? "native" : "web-audio",
    contextSampleRate: state.contextSampleRate,
    playbackRate: preset.rate,
    preservePitch: preset.preservePitch,
    reverbWet: preset.wet,
    presetLabel: processed ? preset.label : null,
    // Phones use the system volume, which is outside the app's path.
    volume: null,
    leveling: state.leveling,
    levelingUnavailable: state.levelingUnavailable,
    dsp: dspAvailable(state.platform) ? state.dsp : null,
    // react-native-audio-api has no channel splitter for crossfeed.
    crossfeedUnsupported: state.platform !== "web",
    output: state.casting
      ? { kind: "cast", name: state.castDeviceName }
      : { kind: "system" },
  };
}
