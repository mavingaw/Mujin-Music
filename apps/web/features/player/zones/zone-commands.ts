import type {
  ZoneCommand,
  ZoneRepeatMode,
  ZoneStatus,
} from "@parson/music-sdk";

/** What this device's player exposes to remote zone commands. */
export interface LocalZoneControls {
  isPlaying: () => boolean;
  isMuted: () => boolean;
  play: () => void;
  pause: () => void;
  next: () => void;
  previous: () => void;
  /** Seconds. */
  seek: (seconds: number) => void;
  jump: (index: number) => void;
  /** 0..100 */
  setVolume: (percent: number) => void;
  setMuted: (muted: boolean) => void;
  load: (
    songIds: string[],
    position: number,
    positionMs: number,
    play: boolean,
  ) => Promise<void>;
  /** Song id at a queue position, to refuse edits made on a stale queue. */
  songIdAt: (index: number) => string | undefined;
  moveQueueItem: (from: number, to: number) => void;
  removeQueueItem: (index: number) => void;
  clearUpcoming: () => void;
  /**
   * Inserts songs right after the current track (`next`) or at the end,
   * keeping everything already queued.
   */
  addSongs: (songIds: string[], next: boolean) => Promise<void>;
  setRepeat: (mode: ZoneRepeatMode) => void;
  setShuffle: (enabled: boolean) => void;
}

/** Applies one remote command to the local player. */
export async function applyZoneCommand(
  command: ZoneCommand,
  controls: LocalZoneControls,
): Promise<void> {
  switch (command.command) {
    case "play":
      if (!controls.isPlaying()) controls.play();
      return;
    case "pause":
    case "stop":
      if (controls.isPlaying()) controls.pause();
      return;
    case "play_pause":
      if (controls.isPlaying()) controls.pause();
      else controls.play();
      return;
    case "next":
      controls.next();
      return;
    case "previous":
      controls.previous();
      return;
    case "seek":
      controls.seek(Math.max(0, command.position_ms) / 1000);
      return;
    case "jump":
      controls.jump(command.queue_position);
      return;
    case "set_volume":
      controls.setVolume(Math.max(0, Math.min(100, command.volume)));
      return;
    case "adjust_volume":
      // The server resolves relative volume for client zones.
      return;
    case "set_mute":
      if (controls.isMuted() !== command.muted)
        controls.setMuted(command.muted);
      return;
    case "load":
      await controls.load(
        command.song_ids,
        command.current_position ?? 0,
        command.position_ms ?? 0,
        command.play ?? true,
      );
      return;
    case "move_queue_item":
      if (
        command.song_id === undefined ||
        controls.songIdAt(command.from) === command.song_id
      ) {
        controls.moveQueueItem(command.from, command.to);
      }
      return;
    case "remove_queue_item":
      if (
        command.song_id === undefined ||
        controls.songIdAt(command.queue_position) === command.song_id
      ) {
        controls.removeQueueItem(command.queue_position);
      }
      return;
    case "clear_upcoming":
      controls.clearUpcoming();
      return;
    case "add_to_queue":
      if (command.song_ids.length)
        await controls.addSongs(command.song_ids, command.next ?? false);
      return;
    case "set_repeat":
      controls.setRepeat(command.mode);
      return;
    case "set_shuffle":
      controls.setShuffle(command.enabled);
      return;
  }
}

export interface LocalPlaybackState {
  songId: string;
  queueSongIds: string[];
  queueIndex: number;
  isPlaying: boolean;
  hasError: boolean;
}

export function localZoneStatus(state: LocalPlaybackState): ZoneStatus {
  if (!state.songId) return "idle";
  if (state.hasError) return "stopped";
  return state.isPlaying ? "playing" : "paused";
}

/** A short key that changes whenever the queue changes. */
export function queueKey(songIds: string[]): string {
  let hash = 0;
  for (const id of songIds) {
    for (let index = 0; index < id.length; index += 1) {
      hash = (Math.imul(hash, 31) + id.charCodeAt(index)) | 0;
    }
    hash = (Math.imul(hash, 31) + 7) | 0;
  }
  return `${songIds.length}:${hash >>> 0}`;
}

/** Report sooner while playing so remotes see fresh positions. */
export function heartbeatInterval(status: ZoneStatus, controlled: boolean) {
  if (controlled) return 3_000;
  return status === "playing" ? 10_000 : 20_000;
}
