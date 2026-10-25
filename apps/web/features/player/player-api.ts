"use client";

import type {
  DspSettings,
  VolumeLevelingMode,
  VolumeLevelingResult,
} from "@parson/music-sdk";
import type { OutputDevice } from "./dsp-store";
import type { Album, Artist, LibrarySong } from "@parson/music-sdk/types";
import { createContext, useContext } from "react";
import type { AudioPreset, AudioPresetId } from "./audio-presets";
import type { PlayerInput, QueueInput, QueueItem } from "./player-model";

export type Player = {
  song: LibrarySong;
  album: Album;
  artist: Artist;
  imageSrc: string;
  queue: QueueItem[];
  isPlaying: boolean;
  error: string | null;
  currentTime: number;
  duration: number;
  volume: number;
  muted: boolean;
  slowedReverb: boolean;
  audioPreset: AudioPresetId;
  audioPresets: readonly AudioPreset[];
  looping: boolean;
  /** What this client does to the stream, for the signal path. */
  signal: PlayerSignalState;
  volumeLevelingMode: VolumeLevelingMode;
  setVolumeLevelingMode: (mode: VolumeLevelingMode) => void;
  /** DSP chain for the current output device. */
  dsp: DspSettings;
  setDsp: (settings: DspSettings) => void;
  output: PlayerOutputState;
  setSongCallback: (
    song: PlayerInput,
    artist?: Partial<Artist>,
    album?: Partial<Album>,
  ) => void;
  setQueue: (queue: QueueInput[]) => void;
  addNextToQueue: (items: QueueInput[]) => void;
  addToQueue: (items: QueueInput[]) => void;
  setCurrentSongIndex: (index: number) => void;
  playAudioSource: () => void;
  togglePlayPause: () => void;
  playNextSong: () => void;
  playPreviousSong: () => void;
  playQueueItem: (index: number) => void;
  reorderQueue: (from: number, to: number) => void;
  handleTimeChange: (value: number | string) => void;
  getPlaybackPosition: () => number;
  setAudioVolume: (value: number | string) => void;
  toggleMute: () => void;
  toggleSlowedReverb: () => void;
  setAudioPreset: (preset: AudioPresetId) => void;
  toggleLoop: () => void;
};

export type PlayerSignalState = {
  /** Bitrate of the loaded stream in kbps, 0 for the original file. */
  streamBitrate: number;
  engine: "media-element" | "web-audio";
  /** `AudioContext.sampleRate` while the Web Audio graph is active. */
  contextSampleRate: number | null;
  leveling: VolumeLevelingResult | null;
  levelingUnavailable: boolean;
  dsp: DspSettings | null;
  /** The output device's own rate, probed with an AudioContext. */
  deviceSampleRate: number | null;
  /** Label of the device audio goes to, when the browser reveals it. */
  outputName: string | null;
  outputChosen: boolean;
};

export type PlayerOutputState = {
  devices: OutputDevice[];
  /** The device the user picked, `""` for the system default. */
  selectedId: string;
  /** The device audio actually goes to. */
  activeId: string;
  label: string | null;
  labelsHidden: boolean;
  /** The browser can route audio to a chosen device (`setSinkId`). */
  selectable: boolean;
  select: (id: string) => void;
  requestLabels: () => Promise<void>;
};

export const PlayerContext = createContext<Omit<Player, "currentTime"> | null>(
  null,
);
export const PlayerClockContext = createContext<number | null>(null);

export function usePlayerClock() {
  const time = useContext(PlayerClockContext);
  if (time === null) {
    throw new Error("usePlayerClock must be used inside PlayerProvider");
  }
  return time;
}

export function usePlayer() {
  const player = useContext(PlayerContext);
  if (!player) {
    throw new Error("usePlayer must be used inside PlayerProvider");
  }
  return player;
}
