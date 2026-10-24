import { describe, expect, test } from "bun:test";
import { buildSignalPath, playerAudioPresets } from "@parson/music-sdk";
import { webClientLeg, type WebSignalInput } from "./signal-path";
import {
  elementLeveling,
  parseVolumeLevelingMode,
  playsInAlbumContext,
} from "./volume-leveling";

const base: WebSignalInput = {
  casting: false,
  desktop: false,
  muted: false,
  preset: playerAudioPresets[0]!,
  signal: {
    contextSampleRate: null,
    engine: "media-element",
    leveling: null,
    levelingUnavailable: false,
    streamBitrate: 0,
  },
  volume: 1,
};

const flac = {
  codec: "flac",
  container: "flac",
  sample_rate: 44_100,
  bit_depth: 16,
  channels: 2,
  bitrate_kbps: 900,
  lossless: true,
};

describe("web client leg", () => {
  test("the original preset adds no processing stages", () => {
    const leg = webClientLeg(base);
    expect(leg).toEqual(
      expect.objectContaining({
        platform: "web",
        playbackRate: 1,
        reverbWet: 0,
        presetLabel: null,
        output: { kind: "system" },
      }),
    );
  });

  test("processed presets carry their speed, pitch and reverb into the path", () => {
    const slow = playerAudioPresets.find((preset) => preset.id === "slow")!;
    const path = buildSignalPath(
      null,
      webClientLeg({
        ...base,
        desktop: true,
        preset: slow,
        signal: {
          ...base.signal,
          engine: "web-audio",
          contextSampleRate: 48_000,
        },
      }),
      flac,
    );
    expect(path.stages.map((stage) => stage.id)).toEqual([
      "source",
      "client-decode",
      "client-resample",
      "client-speed",
      "client-reverb",
      "client-output",
    ]);
    expect(path.stages[1]!.title).toBe("Parson desktop");
    expect(path.stages[3]!.title).toBe("Slow: speed");
  });

  test("casting reports the receiver instead of local processing", () => {
    const leg = webClientLeg({
      ...base,
      casting: true,
      castDeviceName: "Kitchen",
    });
    expect(leg.output).toEqual({ kind: "cast", name: "Kitchen" });
  });
});

describe("element volume leveling", () => {
  test("attenuation is exact and boosts use only the slider's headroom", () => {
    const quieter = elementLeveling({ track_gain_db: -6 }, "track", 1, false);
    expect(quieter.gain).toBeCloseTo(0.501, 3);
    expect(quieter.result?.limitedBy).toBeNull();

    const atFullVolume = elementLeveling(
      { track_gain_db: 4 },
      "track",
      1,
      false,
    );
    expect(atFullVolume.gain).toBe(1);
    expect(atFullVolume.result?.limitedBy).toBe("full_volume");

    const withHeadroom = elementLeveling(
      { track_gain_db: 4 },
      "track",
      0.5,
      false,
    );
    expect(withHeadroom.result?.appliedDb).toBeCloseTo(4, 5);
    expect(0.5 * withHeadroom.gain).toBeLessThanOrEqual(1);
  });

  test("untagged tracks and the off mode leave the volume alone", () => {
    expect(elementLeveling(null, "auto", 1, false)).toEqual({
      result: null,
      gain: 1,
      unavailable: true,
    });
    expect(elementLeveling({ track_gain_db: -3 }, "off", 1, false)).toEqual({
      result: null,
      gain: 1,
      unavailable: false,
    });
  });

  test("auto leveling uses album gain inside album runs", () => {
    expect(playsInAlbumContext(["a", "a", "b"], 0)).toBeTrue();
    expect(playsInAlbumContext(["a", "b", "c"], 1)).toBeFalse();
    expect(playsInAlbumContext(["a"], -1)).toBeFalse();
    const album = elementLeveling(
      { track_gain_db: -8, album_gain_db: -5 },
      "auto",
      1,
      true,
    );
    expect(album.result?.source).toBe("album");
  });

  test("stored modes are validated", () => {
    expect(parseVolumeLevelingMode("album")).toBe("album");
    expect(parseVolumeLevelingMode("loud")).toBe("off");
    expect(parseVolumeLevelingMode(null)).toBe("off");
  });
});
