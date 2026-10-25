import { expect, test } from "bun:test";

import {
  applyMobileZoneCommand,
  mobileZoneName,
  mobileZoneStatus,
  mobileRepeatMode,
  zoneRepeatMode,
  zonePollInterval,
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
  expect(mobileZoneName("  ", null, "ios")).toBe("Parson on iOS");
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
