import { describe, expect, test } from "bun:test";
import {
  effectiveDuration,
  boundedMediaPosition,
  boundedVolume,
  isCurrentTrackGeneration,
  queueIndexForPersistedPosition,
  setMediaPosition,
} from "./player-state";

describe("player numeric boundaries", () => {
  test("rejects non-finite media state", () => {
    expect(boundedMediaPosition(Number.NaN, 120)).toBeNull();
    expect(boundedMediaPosition(Number.POSITIVE_INFINITY, 120)).toBeNull();
    expect(boundedVolume("not-a-number")).toBeNull();
  });

  test("clamps seeks and volume to browser-supported ranges", () => {
    expect(boundedMediaPosition(-5, 120)).toBe(0);
    expect(boundedMediaPosition(180, 120)).toBe(120);
    expect(boundedVolume(-20)).toBe(0);
    expect(boundedVolume(250)).toBe(1);
    expect(boundedVolume(50)).toBe(0.5);
    expect(boundedMediaPosition("12.5", 120)).toBe(12.5);
    expect(boundedMediaPosition(180, Number.NaN)).toBe(180);
  });

  test("supports very long tracks and contains media-element seek failures", () => {
    const longDuration = 60 * 60 * 24 * 365;
    const media = { currentTime: 0 };
    expect(setMediaPosition(media, longDuration - 0.001, longDuration)).toBe(
      longDuration - 0.001,
    );
    expect(media.currentTime).toBe(longDuration - 0.001);

    const unavailableMedia = {
      get currentTime() {
        return 0;
      },
      set currentTime(_value: number) {
        throw new DOMException("not seekable", "InvalidStateError");
      },
    };
    expect(setMediaPosition(unavailableMedia, 0.001, longDuration)).toBeNull();
  });
});

test("persisted queue positions survive filtered missing songs", () => {
  const positions = [0, 2, 3];
  expect(queueIndexForPersistedPosition(positions, 2)).toBe(1);
  expect(queueIndexForPersistedPosition(positions, 1)).toBe(1);
  expect(queueIndexForPersistedPosition(positions, 99)).toBe(2);
  expect(queueIndexForPersistedPosition([], 4)).toBe(0);
  expect(queueIndexForPersistedPosition([null, null], 4)).toBe(1);
});

describe("async playback generations", () => {
  test("accepts only work for the active generation and track", () => {
    expect(isCurrentTrackGeneration(4, 4, "song-a", "song-a")).toBeTrue();
    expect(isCurrentTrackGeneration(3, 4, "song-a", "song-a")).toBeFalse();
    expect(isCurrentTrackGeneration(4, 4, "song-a", "song-b")).toBeFalse();
    expect(isCurrentTrackGeneration(4, 4)).toBeTrue();
  });
});

describe("effectiveDuration", () => {
  test("a converted stream with no length shows the library's length", () => {
    // ALAC 24/192 converted to FLAC while it plays: Infinity, then NaN.
    expect(effectiveDuration(Infinity, 35.2)).toBe(35.2);
    expect(effectiveDuration(Number.NaN, 35.2)).toBe(35.2);
    expect(effectiveDuration(0, 35.2)).toBe(35.2);
    // Only the first seconds have arrived.
    expect(effectiveDuration(4.1, 35.2)).toBe(35.2);
  });

  test("a real media length wins", () => {
    expect(effectiveDuration(35.0, 35.2)).toBe(35.0);
    expect(effectiveDuration(240, undefined)).toBe(240);
    expect(effectiveDuration(Infinity, undefined)).toBe(0);
    expect(effectiveDuration(Number.NaN, 0)).toBe(0);
  });
});
