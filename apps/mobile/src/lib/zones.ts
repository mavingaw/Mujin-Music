import type { ZoneCommand, ZoneStatus } from "@parson/music-sdk";

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
    default:
      return false;
  }
}

/** Polling cadence: fast while the zone sheet is open or a remote acts. */
export function zonePollInterval(sheetOpen: boolean, playing: boolean) {
  if (sheetOpen) return 2_000;
  return playing ? 5_000 : 15_000;
}
