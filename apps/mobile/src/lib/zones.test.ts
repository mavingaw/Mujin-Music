import { expect, test } from "bun:test";
import type { Zone } from "@parson/music-sdk";

import {
  applyMobileZoneCommand,
  mobileZoneName,
  mobileZoneStatus,
  mobileRepeatMode,
  zoneRepeatMode,
  zonePollInterval,
  signedLabel,
  steppedValue,
  mobilePlaybackTarget,
  type MobileZoneControls,
} from "./zones";

function controls(playing: boolean) {
  const calls: string[] = [];
  const value: MobileZoneControls = {
    isPlaying: () => playing,
    toggle: () => calls.push("toggle"),
    next: () => calls.push("next"),
    previous: () => calls.push("previous"),
    seek: (seconds) => calls.push(`seek:${seconds}`),
    playAt: (index) => calls.push(`at:${index}`),
    load: async (ids, position, positionMs) => {
      calls.push(`load:${ids.length}@${position}+${positionMs}`);
    },
    songIdAt: (index) => ["a", "b", "c"][index],
    move: (from, to) => calls.push(`move:${from}->${to}`),
    remove: (index) => calls.push(`remove:${index}`),
    clearUpcoming: () => calls.push("clear"),
    addSongs: async (ids, next) => {
      calls.push(`add:${ids.join(",")}${next ? ":next" : ""}`);
    },
    setRepeat: (mode) => calls.push(`repeat:${mode}`),
    setShuffle: (enabled) => calls.push(`shuffle:${enabled}`),
  };
  return { calls, value };
}

test("phones name themselves after the device", () => {
  expect(mobileZoneName("Alex's Pixel", "Pixel 9", "android")).toBe(
    "Alex's Pixel",
  );
  expect(mobileZoneName(null, "Pixel 9", "android")).toBe("Pixel 9 (Android)");
  expect(mobileZoneName("  ", null, "ios")).toBe("Mujin Music on iOS");
});

test("remote commands drive the native player", async () => {
  const paused = controls(false);
  expect(await applyMobileZoneCommand({ command: "play" }, paused.value)).toBe(
    true,
  );
  await applyMobileZoneCommand({ command: "pause" }, paused.value);
  await applyMobileZoneCommand(
    { command: "seek", position_ms: 61_000 },
    paused.value,
  );
  await applyMobileZoneCommand(
    { command: "jump", queue_position: 2 },
    paused.value,
  );
  await applyMobileZoneCommand(
    { command: "load", song_ids: ["a", "b"], current_position: 1 },
    paused.value,
  );
  expect(paused.calls).toEqual(["toggle", "seek:61", "at:2", "load:2@1+0"]);
  expect(
    await applyMobileZoneCommand(
      { command: "set_volume", volume: 10 },
      paused.value,
    ),
  ).toBe(false);
});

test("status and polling cadence", () => {
  expect(mobileZoneStatus(false, false, false)).toBe("idle");
  expect(mobileZoneStatus(true, true, true)).toBe("buffering");
  expect(mobileZoneStatus(true, false, false)).toBe("paused");
  expect(zonePollInterval(true, false)).toBeLessThan(
    zonePollInterval(false, true),
  );
});

test("remote queue management reaches the phone's queue", async () => {
  const phone = controls(true);
  await applyMobileZoneCommand(
    { command: "move_queue_item", from: 2, to: 1, song_id: "c" },
    phone.value,
  );
  await applyMobileZoneCommand(
    { command: "remove_queue_item", queue_position: 2, song_id: "stale" },
    phone.value,
  );
  await applyMobileZoneCommand(
    { command: "remove_queue_item", queue_position: 2, song_id: "c" },
    phone.value,
  );
  await applyMobileZoneCommand({ command: "clear_upcoming" }, phone.value);
  await applyMobileZoneCommand(
    { command: "set_repeat", mode: "off" },
    phone.value,
  );
  await applyMobileZoneCommand(
    { command: "set_shuffle", enabled: false },
    phone.value,
  );
  expect(phone.calls).toEqual([
    "move:2->1",
    "remove:2",
    "clear",
    "repeat:none",
    "shuffle:false",
  ]);
  expect(zoneRepeatMode("none")).toBe("off");
  expect(mobileRepeatMode("one")).toBe("one");
});

test("settings steppers stay inside their range", () => {
  expect(steppedValue(-18, 1, -30, -5)).toBe(-17);
  expect(steppedValue(-5, 1, -30, -5)).toBe(-5);
  expect(steppedValue(2, -5, 0, 100)).toBe(0);
  expect(signedLabel(2, "dB")).toBe("+2 dB");
  expect(signedLabel(-3, "dB")).toBe("-3 dB");
  expect(signedLabel(0, "dB")).toBe("0 dB");
});

test("library actions follow the picked zone and remote adds reach the phone", async () => {
  const { value, calls } = controls(true);
  await applyMobileZoneCommand(
    { command: "add_to_queue", song_ids: ["x", "y"], next: true },
    value,
  );
  await applyMobileZoneCommand(
    { command: "add_to_queue", song_ids: ["z"] },
    value,
  );
  expect(calls).toEqual(["add:x,y:next", "add:z"]);

  const kitchen = {
    id: "sonos:RINCON_1",
    name: "Kitchen",
    capabilities: { transfer_target: true, queue_edit: true },
    queue: { song_ids: ["a"], current_position: 0 },
  } as unknown as Zone;
  expect(mobilePlaybackTarget(null, "client:phone", [kitchen], true)).toEqual({
    kind: "local",
  });
  expect(
    mobilePlaybackTarget("client:phone", "client:phone", [kitchen], true),
  ).toEqual({ kind: "local" });
  expect(
    mobilePlaybackTarget("sonos:RINCON_1", "client:phone", [kitchen], true),
  ).toMatchObject({ kind: "zone", zoneId: "sonos:RINCON_1", name: "Kitchen" });
  // Still loading: keep the picked zone rather than play on the phone.
  expect(mobilePlaybackTarget("sonos:RINCON_1", null, [], false)).toMatchObject(
    { kind: "zone", zoneId: "sonos:RINCON_1" },
  );
  // Gone: back to the phone.
  expect(mobilePlaybackTarget("sonos:GONE", null, [kitchen], true)).toEqual({
    kind: "local",
  });
});
