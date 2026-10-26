import type { ZoneDspChain } from "./zone-dsp";
import {
  type DspChain,
  defaultProcessedAudioPreset,
  fillReverbImpulseChannel,
  getPlayerAudioPreset,
  playerAudioPresets,
  reverbImpulseLength,
  type PlayerAudioPreset,
  type PlayerAudioPresetId,
} from "@parson/music-sdk";

export type AudioPresetId = PlayerAudioPresetId;
export type AudioPreset = PlayerAudioPreset;

export type AudioGraph = {
  /** Headroom, EQ and crossfeed between the source and the preset mix. */
  chain: DspChain;
  /** Rate asked of the context, `null` for the device's own. */
  requestedRate: number | null;
  convolver: ConvolverNode;
  context: AudioContext;
  dryGain: GainNode;
  source: MediaElementAudioSourceNode;
  wetGain: GainNode;
  wetConnected: boolean;
  /** Per-zone DSP between the mix and the speakers. */
  zoneDsp?: ZoneDspChain;
};

type PitchControllableMedia = {
  preservesPitch: boolean;
  webkitPreservesPitch?: boolean;
  mozPreservesPitch?: boolean;
};

export function setPitchPreservation(
  element: PitchControllableMedia,
  preservePitch: boolean,
) {
  element.preservesPitch = preservePitch;
  if ("webkitPreservesPitch" in element) {
    element.webkitPreservesPitch = preservePitch;
  }
  if ("mozPreservesPitch" in element) {
    element.mozPreservesPitch = preservePitch;
  }
}

export const audioPresets = playerAudioPresets;

export const defaultAudioPreset = defaultProcessedAudioPreset;

export function getAudioPreset(id: AudioPresetId) {
  return getPlayerAudioPreset(id);
}

export function createAudioElement() {
  const element = new Audio();
  element.crossOrigin = "anonymous";
  element.preload = "auto";
  return element;
}

export function buildReverbImpulse(context: AudioContext) {
  // Keep convolution short for WebKitGTK.
  const length = reverbImpulseLength(context.sampleRate);
  const impulse = context.createBuffer(2, length, context.sampleRate);

  for (let channel = 0; channel < impulse.numberOfChannels; channel += 1) {
    fillReverbImpulseChannel(impulse.getChannelData(channel), channel);
  }

  return impulse;
}
