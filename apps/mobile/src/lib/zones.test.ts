import { expect, test } from "bun:test";

import {
  applyMobileZoneCommand,
  mobileZoneName,
  mobileZoneStatus,
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
