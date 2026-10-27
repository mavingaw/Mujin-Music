"use client";

import type {
  DspSettings,
  RadioSeed,
  RadioSkipReasonId,
  ResolvedRadioSeed,
  VolumeLevelingMode,
  VolumeLevelingResult,
  ZoneRepeatMode,
} from "@parson/music-sdk";
import type { OutputDevice } from "./dsp-store";
import type { Album, Artist, LibrarySong } from "@parson/music-sdk/types";
import { createContext, useContext } from "react";
import type { AudioPreset, AudioPresetId } from "./audio-presets";
import type { PlayerInput, QueueInput, QueueItem } from "./player-model";
import type { PendingRadioSkip } from "./player-controller";

export type PlayerRadio = {
  /** Continue with Radio picks once the queue runs out. */
  enabled: boolean;
  /** Show the "Coming up" notice before a Radio pick plays. */
  comingUpNotice: boolean;
  /** What the current station is built around, when known. */
  seed: ResolvedRadioSeed | null;
  /** A Radio pick skipped early, awaiting an optional reason. */
  pendingSkip: PendingRadioSkip | null;
  answerSkip: (reason: RadioSkipReasonId) => void;
  dismissSkip: () => void;
  setEnabled: (enabled: boolean) => void;
  setComingUpNotice: (visible: boolean) => void;
  /**
   * Starts a station. Resolves to the station's display name, or null when
   * the library has nothing to play for it.
   */
  start: (seed: RadioSeed) => Promise<string | null>;
  /** Thumbs up a queue item (current or upcoming). */
  thumbsUp: (index: number) => void;
  /** Thumbs down: skips the current item, or replaces an upcoming pick. */
  thumbsDown: (index: number) => void;
};

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
  /**
   * The stream now playing: its URL and how many times its first frame
   * has been decoded (0 until the first).
   */
  stream: { url: string; frames: number };
  audioPresets: readonly AudioPreset[];
  /** True while repeating the current track (`repeatMode === "one"`). */
  looping: boolean;
  /** Index of the active item in `queue`. */
  currentIndex: number;
  radio: PlayerRadio;
  /** What this client does to the stream, for the signal path. */
  signal: PlayerSignalState;
  volumeLevelingMode: VolumeLevelingMode;
  setVolumeLevelingMode: (mode: VolumeLevelingMode) => void;
  /**
   * The DSP chain this browser runs: its zone's chain (`setZoneDsp`), or,
   * before the zone list arrives, the chain kept in this browser.
   */
  dsp: DspSettings;
  /** The chain kept in this browser (used while there is no zone). */
  browserDsp: DspSettings;
  setDsp: (settings: DspSettings) => void;
  /** This device's zone chain, as the server keeps it; `null`: no zone. */
  setZoneDsp: (settings: DspSettings | null) => void;
  output: PlayerOutputState;
  repeatMode: ZoneRepeatMode;
  shuffle: boolean;
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
  /** Removes an upcoming item (not history or the current track). */
  removeQueueItem: (index: number) => void;
  clearUpcoming: () => void;
  setRepeatMode: (mode: ZoneRepeatMode) => void;
  /** Shuffles upcoming items; turning it off restores their order. */
  setShuffle: (enabled: boolean) => void;
  handleTimeChange: (value: number | string) => void;
  getPlaybackPosition: () => number;
  setAudioVolume: (value: number | string) => void;
  /** This zone's crossfade time (s) and leveling factor for the current track. */
  setOutputSettings: (settings: {
    crossfadeSeconds: number;
    /** The zone's leveling factor; `null` when the zone does not level. */
    levelingFactor: number | null;
  }) => void;
  toggleMute: () => void;
  toggleSlowedReverb: () => void;
  setAudioPreset: (preset: AudioPresetId) => void;
  /** Cycles repeat: off, queue, track. */
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
  /** The convolution filter the browser chain runs, once known. */
  convolution?: { name: string; loaded: boolean } | null;
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
