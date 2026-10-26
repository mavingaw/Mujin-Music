import { expect, test } from "bun:test";
import {
  crossfadeGains,
  leveledVolume,
  levelingFactor,
  shouldStartCrossfade,
} from "./crossfade";

const base = {
  seconds: 6,
  currentTime: 0,
  duration: 200,
  repeatOne: false,
  hasNext: true,
  alreadyStarted: false,
};

test("crossfade starts within its window before the track ends", () => {
  expect(shouldStartCrossfade({ ...base, currentTime: 190 })).toBe(false);
  expect(shouldStartCrossfade({ ...base, currentTime: 194.5 })).toBe(true);
  expect(shouldStartCrossfade({ ...base, currentTime: 199.99 })).toBe(false);
});

test("crossfade stays off when it cannot or should not run", () => {
  const near = { ...base, currentTime: 197 };
  expect(shouldStartCrossfade({ ...near, seconds: 0 })).toBe(false);
  expect(shouldStartCrossfade({ ...near, repeatOne: true })).toBe(false);
  expect(shouldStartCrossfade({ ...near, hasNext: false })).toBe(false);
  expect(shouldStartCrossfade({ ...near, alreadyStarted: true })).toBe(false);
  expect(shouldStartCrossfade({ ...near, duration: Number.NaN })).toBe(false);
  // A 9-second jingle crossfades over 3 seconds at most.
  expect(shouldStartCrossfade({ ...base, duration: 9, currentTime: 5 })).toBe(
    false,
  );
  expect(shouldStartCrossfade({ ...base, duration: 9, currentTime: 6.5 })).toBe(
    true,
  );
});

test("crossfade keeps equal power", () => {
  const start = crossfadeGains(0);
  expect(start).toEqual({ outgoing: 1, incoming: 0 });
  const middle = crossfadeGains(0.5);
  expect(middle.outgoing ** 2 + middle.incoming ** 2).toBeCloseTo(1, 6);
  const end = crossfadeGains(2);
  expect(end.outgoing).toBeCloseTo(0, 6);
  expect(end.incoming).toBe(1);
});

test("leveling lowers loud tracks and raises quiet ones within the volume", () => {
  expect(levelingFactor(-6)).toBeCloseTo(0.501, 3);
  expect(levelingFactor(null)).toBe(1);
  expect(leveledVolume(0.8, levelingFactor(-6))).toBeCloseTo(0.401, 3);
  expect(leveledVolume(0.5, levelingFactor(6))).toBeCloseTo(0.998, 3);
  expect(leveledVolume(0.9, levelingFactor(6))).toBe(1);
});
