import type {
  ZoneCommand,
  ZoneRepeatMode,
  ZoneStatus,
} from "@parson/music-sdk";

/** The phone player's repeat names ("none") vs the zone API's ("off"). */
export type MobileRepeatMode = "none" | "one" | "all";

export function zoneRepeatMode(mode: MobileRepeatMode): ZoneRepeatMode {
  return mode === "none" ? "off" : mode;
}

export function mobileRepeatMode(mode: ZoneRepeatMode): MobileRepeatMode {
  return mode === "off" ? "none" : mode;
}

export const ZONE_DEVICE_KEY = "parson.zones.device-id";
export const ZONE_SELECTED_KEY = "parson.zones.selected";

/** "Alex's Pixel" or "Pixel 9 (Android)". */
export function mobileZoneName(
  deviceName: string | null | undefined,
  modelName: string | null | undefined,
  os: string,
): string {
  const name = deviceName?.trim();
  if (name) return name.slice(0, 80);
  const model = modelName?.trim();
  const platform = os === "ios" ? "iOS" : os === "android" ? "Android" : os;
  return model ? `${model} (${platform})` : `Parson on ${platform}`;
}

export function mobileZoneStatus(
  hasSong: boolean,
  isPlaying: boolean,
  isBuffering: boolean,
): ZoneStatus {
  if (!hasSong) return "idle";
  if (isBuffering && isPlaying) return "buffering";
  return isPlaying ? "playing" : "paused";
}

/** What the phone's player can do for a remote command. */
export interface MobileZoneControls {
  isPlaying: () => boolean;
  toggle: () => void;
  next: () => void;
  previous: () => void;
  seek: (seconds: number) => void;
  playAt: (index: number) => void;
  load: (
    songIds: string[],
    position: number,
    positionMs: number,
    play: boolean,
  ) => Promise<void>;
  /** Song id at a queue position, to refuse edits made on a stale queue. */
  songIdAt: (index: number) => string | undefined;
  move: (from: number, to: number) => void;
  remove: (index: number) => void;
  clearUpcoming: () => void;
  setRepeat: (mode: MobileRepeatMode) => void;
  setShuffle: (enabled: boolean) => void;
}

/**
 * Applies a remote command. Phones use the system volume, so volume and
 * mute commands are left to the hardware buttons.
 */
export async function applyMobileZoneCommand(
  command: ZoneCommand,
  controls: MobileZoneControls,
): Promise<boolean> {
  switch (command.command) {
    case "play":
      if (!controls.isPlaying()) controls.toggle();
      return true;
    case "pause":
    case "stop":
      if (controls.isPlaying()) controls.toggle();
      return true;
    case "play_pause":
      controls.toggle();
      return true;
    case "next":
      controls.next();
      return true;
    case "previous":
      controls.previous();
      return true;
    case "seek":
      controls.seek(Math.max(0, command.position_ms) / 1000);
      return true;
    case "jump":
      controls.playAt(command.queue_position);
      return true;
    case "load":
      await controls.load(
        command.song_ids,
        command.current_position ?? 0,
        command.position_ms ?? 0,
        command.play ?? true,
      );
      return true;
    case "move_queue_item":
      if (
        command.song_id === undefined ||
        controls.songIdAt(command.from) === command.song_id
      ) {
        controls.move(command.from, command.to);
      }
      return true;
    case "remove_queue_item":
      if (
        command.song_id === undefined ||
        controls.songIdAt(command.queue_position) === command.song_id
      ) {
        controls.remove(command.queue_position);
      }
      return true;
    case "clear_upcoming":
      controls.clearUpcoming();
      return true;
    case "set_repeat":
      controls.setRepeat(mobileRepeatMode(command.mode));
      return true;
    case "set_shuffle":
      controls.setShuffle(command.enabled);
      return true;
    default:
      return false;
  }
}

/** Polling cadence: fast while the zone sheet is open or a remote acts. */
export function zonePollInterval(sheetOpen: boolean, playing: boolean) {
  if (sheetOpen) return 2_000;
  return playing ? 5_000 : 15_000;
}

/** A stepper's next value, kept inside `min`..`max`. */
export function steppedValue(
  value: number,
  delta: number,
  min: number,
  max: number,
): number {
  return Math.min(max, Math.max(min, Math.round(value + delta)));
}

/** "+2 dB", "0 dB", "-3 dB". */
export function signedLabel(value: number, unit: string): string {
  return `${value > 0 ? "+" : ""}${value} ${unit}`;
}
