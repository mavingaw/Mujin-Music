"use client";

import type { ZoneRepeatMode } from "@parson/music-sdk";
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
  /** True while repeating the current track (`repeatMode === "one"`). */
  looping: boolean;
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
  toggleMute: () => void;
  toggleSlowedReverb: () => void;
  setAudioPreset: (preset: AudioPresetId) => void;
  /** Cycles repeat: off, queue, track. */
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
