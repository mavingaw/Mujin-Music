import { describe, expect, test } from "bun:test";
import { getPlayerAudioPreset } from "@parson/music-sdk";
import {
  formatPlaybackTime,
  timelinePlaybackRate,
  timelineValueText,
} from "./player-controls-state";

describe("player control presentation", () => {
  test("formats playback times safely", () => {
    expect(formatPlaybackTime(Number.NaN)).toBe("0:00");
    expect(formatPlaybackTime(-1)).toBe("0:00");
    expect(formatPlaybackTime(65.9)).toBe("1:05");
  });

  test("provides a useful spoken timeline value", () => {
    expect(timelineValueText(65, 183)).toBe("1:05 of 3:03");
  });

  test("a remote zone's length is never scaled by this device's preset", () => {
    // Regression: a 3:20 track on a Sonos zone (or a group of zones) read
    // 4:16 while this browser had the slowed preset (rate 0.78) selected.
    const slowed = getPlayerAudioPreset("slow").rate;
    expect(slowed).toBeLessThan(1);
    const trackSeconds = 200;
    const remoteRate = timelinePlaybackRate(slowed, true);
    expect(formatPlaybackTime(trackSeconds / remoteRate)).toBe("3:20");
    // The local player still shows its slowed length.
    expect(
      formatPlaybackTime(trackSeconds / timelinePlaybackRate(slowed, false)),
    ).toBe("4:16");
    expect(timelinePlaybackRate(0, false)).toBe(1);
  });
});
