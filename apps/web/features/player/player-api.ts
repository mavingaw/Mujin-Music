"use client";

import type {
  RadioSeed,
  RadioSkipReasonId,
  ResolvedRadioSeed,
} from "@parson/music-sdk";
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
  audioPresets: readonly AudioPreset[];
  looping: boolean;
  /** Index of the active item in `queue`. */
  currentIndex: number;
  radio: PlayerRadio;
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
