import { describe, expect, test } from "bun:test";
import {
  buildSignalPath,
  defaultDspSettings,
  eqPresets,
  getPlayerAudioPreset,
} from "@parson/music-sdk";
import {
  dspAvailable,
  mobileClientLeg,
  parseStoredDsp,
  nativeLeveling,
  parseVolumeLevelingMode,
  playsInAlbumContext,
  signalPlatform,
  type MobileSignalState,
} from "./signal-path";

const state: MobileSignalState = {
  platform: "android",
  localFile: false,
  streamBitrate: 0,
  contextSampleRate: null,
  leveling: null,
  levelingUnavailable: false,
  casting: false,
  castDeviceName: null,
  dsp: null,
};

const flac = {
  codec: "flac",
  container: "flac",
  sample_rate: 96_000,
  bit_depth: 24,
  channels: 2,
  bitrate_kbps: 3_000,
  lossless: true,
};

describe("mobile signal path", () => {
  test("Android plays through ExoPlayer and the system volume", () => {
    const leg = mobileClientLeg(state, getPlayerAudioPreset("original"));
    expect(leg).toEqual(
      expect.objectContaining({ engine: "native", volume: null }),
    );
    const path = buildSignalPath(null, { ...leg, localFile: true }, flac);
    expect(path.stages.map((stage) => stage.id)).toEqual([
      "source",
      "local-file",
      "client-decode",
      "client-output",
    ]);
    expect(path.stages[3]!.detail).toBe("Android audio mixer");
  });

  test("iOS reports the audio graph rate and preset processing", () => {
    const path = buildSignalPath(
      null,
      mobileClientLeg(
        { ...state, platform: "ios", contextSampleRate: 48_000 },
        getPlayerAudioPreset("deep-slow"),
      ),
      flac,
    );
    expect(path.stages.map((stage) => stage.id)).toEqual([
      "source",
      "client-decode",
      "client-depth-in",
      "client-speed",
      "client-resample",
      "client-reverb",
      "client-depth-out",
      "client-output",
    ]);
    expect(path.stages[1]!.title).toBe("This iPhone");
  });

  test("the iOS DSP chain shows headroom and EQ, and skips crossfeed", () => {
    const dsp = defaultDspSettings();
    dsp.headroom = { enabled: true, gainDb: -5 };
    dsp.eq = {
      enabled: true,
      preset: "bass",
      bands: eqPresets.find((preset) => preset.id === "bass")!.bands,
    };
    dsp.crossfeed.enabled = true;
    const path = buildSignalPath(
      null,
      mobileClientLeg(
        { ...state, platform: "ios", contextSampleRate: 96_000, dsp },
        getPlayerAudioPreset("original"),
      ),
      flac,
    );
    const stage = (id: string) => path.stages.find((item) => item.id === id);
    expect(stage("dsp-headroom")?.detail).toBe("−5.0 dB");
    expect(stage("dsp-eq")?.detail).toStartWith("2 bands");
    expect(stage("dsp-crossfeed")).toEqual(
      expect.objectContaining({
        detail: "Not available on this device",
        quality: null,
      }),
    );
    // The chain itself is enhanced; the iOS audio session is a shared
    // mixer, which keeps the whole path at high quality.
    expect(
      path.stages
        .filter((item) => item.kind !== "output" && item.quality)
        .every((item) => item.quality !== "high_quality"),
    ).toBeTrue();
    expect(stage("dsp-eq")?.quality).toBe("enhanced");
    expect(stage("client-output")?.quality).toBe("high_quality");
    expect(path.quality).toBe("high_quality");

    const android = mobileClientLeg(
      { ...state, dsp },
      getPlayerAudioPreset("original"),
    );
    expect(android.dsp).toBeNull();
    expect(dspAvailable("android")).toBeFalse();
  });

  test("stored DSP settings are parsed safely", () => {
    expect(parseStoredDsp(null)).toEqual(defaultDspSettings());
    expect(parseStoredDsp("{oops")).toEqual(defaultDspSettings());
    expect(
      parseStoredDsp(
        JSON.stringify({ headroom: { enabled: true, gainDb: -2 } }),
      ).headroom,
    ).toEqual({ enabled: true, gainDb: -2 });
  });

  test("a downloaded copy is not reported while casting", () => {
    const leg = mobileClientLeg(
      { ...state, localFile: true, casting: true, castDeviceName: "Den" },
      getPlayerAudioPreset("original"),
    );
    expect(leg.localFile).toBeFalse();
    expect(leg.output).toEqual({ kind: "cast", name: "Den" });
  });

  test("Android leveling can only attenuate; the graph can boost", () => {
    const boost = { track_gain_db: 3, track_peak: 0.5 };
    const android = nativeLeveling(boost, "track", "android", false);
    expect(android.gain).toBe(1);
    expect(android.result?.limitedBy).toBe("full_volume");
    const ios = nativeLeveling(boost, "track", "ios", false);
    expect(ios.gain).toBeCloseTo(10 ** (3 / 20), 5);
    expect(nativeLeveling(null, "track", "ios", false).unavailable).toBeTrue();
    expect(nativeLeveling(boost, "off", "ios", false).gain).toBe(1);
  });

  test("helpers parse platform, mode and album context", () => {
    expect(signalPlatform("ios")).toBe("ios");
    expect(signalPlatform("macos")).toBe("web");
    expect(parseVolumeLevelingMode("auto")).toBe("auto");
    expect(parseVolumeLevelingMode("max")).toBe("off");
    expect(playsInAlbumContext(["a", "a"], 1)).toBeTrue();
    expect(playsInAlbumContext([undefined, undefined], 0)).toBeFalse();
  });
});
