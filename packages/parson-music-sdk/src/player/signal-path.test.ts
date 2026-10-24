import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import { clearCachedSignalPaths, getSongSignalPath } from "../resources/songs";
import {
  buildSignalPath,
  describeAudioFormat,
  describeCollectionFormat,
  formatDecibels,
  overallSignalQuality,
  signalPathSummary,
  sourceQuality,
  volumeLevelingGain,
  type ClientSignalLeg,
  type ServerSignalPath,
  type SignalFormat,
} from "./signal-path";

const hiRes: SignalFormat = {
  codec: "flac",
  container: "flac",
  sample_rate: 96_000,
  bit_depth: 24,
  channels: 2,
  bitrate_kbps: 3_000,
  lossless: true,
  high_res: true,
  summary: "FLAC 96kHz 24bit 2ch",
};

function originalServer(format = hiRes): ServerSignalPath {
  return {
    song_id: "song",
    target: "direct",
    mode: "original",
    quality: format.lossless ? "lossless" : "low_quality",
    source: format,
    delivered: format,
    content_type: "audio/flac",
    requested_bitrate_kbps: 0,
    effect: null,
    stages: [
      {
        id: "source",
        kind: "source",
        title: "Source",
        detail: format.summary,
        quality: format.lossless ? "lossless" : "low_quality",
        explanation: "",
      },
      {
        id: "server",
        kind: "transport",
        title: "Parson server",
        detail: "Original file",
        quality: "lossless",
        explanation: "",
      },
    ],
  };
}

const plainBrowser: ClientSignalLeg = {
  platform: "web",
  engine: "media-element",
  playbackRate: 1,
  preservePitch: true,
  reverbWet: 0,
  volume: 1,
  muted: false,
  output: { kind: "system" },
};

const ids = (stages: { id: string }[]) => stages.map((stage) => stage.id);

describe("signal path classification", () => {
  test("an untouched lossless stream is only limited by the OS mixer", () => {
    const path = buildSignalPath(originalServer(), plainBrowser);
    expect(ids(path.stages)).toEqual([
      "source",
      "server",
      "client-decode",
      "client-output",
    ]);
    expect(path.quality).toBe("high_quality");
    expect(path.bitPerfectToOutput).toBeTrue();
    expect(path.highRes).toBeTrue();
    expect(signalPathSummary(path)).toBe("FLAC 96kHz 24bit 2ch");
  });

  test("Cast receivers keep the stream lossless end to end", () => {
    const path = buildSignalPath(originalServer(), {
      ...plainBrowser,
      volume: 0.4,
      playbackRate: 0.78,
      reverbWet: 0.18,
      output: { kind: "cast", name: "Living Room" },
    });
    expect(path.quality).toBe("lossless");
    expect(ids(path.stages)).toEqual([
      "source",
      "server",
      "client-decode",
      "client-output",
    ]);
    expect(path.stages[2]!.title).toBe("Living Room");
  });

  test("presets and leveling make the path enhanced, volume makes it high quality", () => {
    const presets = buildSignalPath(originalServer(), {
      ...plainBrowser,
      engine: "web-audio",
      contextSampleRate: 48_000,
      playbackRate: 0.78,
      preservePitch: false,
      reverbWet: 0.18,
      presetLabel: "Slow",
    });
    expect(ids(presets.stages)).toEqual([
      "source",
      "server",
      "client-decode",
      "client-resample",
      "client-speed",
      "client-reverb",
      "client-output",
    ]);
    expect(presets.stages[3]!.detail).toBe("96kHz to 48kHz");
    expect(presets.stages[4]!.detail).toBe("0.78x, pitch follows speed");
    expect(presets.stages[5]!.detail).toBe("Convolution, 1.8s room, 18% wet");
    expect(presets.quality).toBe("high_quality");
    expect(presets.bitPerfectToOutput).toBeFalse();

    const quiet = buildSignalPath(originalServer(), {
      ...plainBrowser,
      volume: 0.5,
    });
    expect(quiet.stages.find((stage) => stage.id === "client-volume")).toEqual(
      expect.objectContaining({ detail: "−6.0 dB", quality: "high_quality" }),
    );
  });

  test("a lossy delivery is low quality whatever the client does", () => {
    const mp3: SignalFormat = {
      ...hiRes,
      codec: "mp3",
      container: "mp3",
      bit_depth: null,
      lossless: false,
      high_res: false,
      bitrate_kbps: 320,
      summary: "MP3 44.1kHz 2ch 320kbps",
    };
    const path = buildSignalPath(originalServer(mp3), plainBrowser);
    expect(path.quality).toBe("low_quality");
  });

  test("offline copies and unknown servers fall back to the song's audio", () => {
    const audio = {
      codec: "alac",
      container: "mp4",
      sample_rate: 44_100,
      bit_depth: 16,
      channels: 2,
      bitrate_kbps: 900,
      lossless: true,
    };
    const local = buildSignalPath(
      null,
      {
        ...plainBrowser,
        platform: "android",
        engine: "native",
        localFile: true,
        volume: null,
      },
      audio,
    );
    expect(ids(local.stages)).toEqual([
      "source",
      "local-file",
      "client-decode",
      "client-output",
    ]);
    expect(local.stages[0]!.title).toBe("Source (downloaded)");
    expect(local.stages[2]!.detail).toBe("Media3 ExoPlayer");
    expect(local.serverKnown).toBeTrue();

    const unknown = buildSignalPath(null, plainBrowser, null);
    expect(unknown.serverKnown).toBeFalse();
    expect(unknown.stages[0]!.quality).toBeNull();
    expect(unknown.quality).toBe("high_quality");
  });

  test("an unclassified source can never be called lossless", () => {
    expect(
      overallSignalQuality([
        {
          id: "source",
          kind: "source",
          title: "",
          detail: "",
          quality: null,
          explanation: "",
        },
        {
          id: "out",
          kind: "output",
          title: "",
          detail: "",
          quality: "lossless",
          explanation: "",
        },
      ]),
    ).toBe("high_quality");
  });

  test("format descriptions flag high resolution and lossy bitrates", () => {
    expect(
      describeAudioFormat({
        codec: "flac",
        container: "flac",
        sample_rate: 44_100,
        bit_depth: 24,
        channels: 2,
        bitrate_kbps: 1_200,
        lossless: true,
      }),
    ).toEqual(
      expect.objectContaining({
        high_res: true,
        summary: "FLAC 44.1kHz 24bit 2ch",
      }),
    );
    expect(
      describeAudioFormat({
        codec: "aac",
        container: "mp4",
        sample_rate: 48_000,
        bit_depth: null,
        channels: 2,
        bitrate_kbps: 256,
        lossless: false,
      })?.summary,
    ).toBe("AAC 48kHz 2ch 256kbps");
    expect(sourceQuality(null)).toBeNull();
    expect(formatDecibels(-6.44)).toBe("−6.4 dB");
    expect(formatDecibels(2)).toBe("+2.0 dB");
  });
});

describe("collection formats", () => {
  const track = (codec: string, rate: number, depth: number | null) => ({
    codec,
    container: codec,
    sample_rate: rate,
    bit_depth: depth,
    channels: 2,
    bitrate_kbps: 900,
    lossless: codec !== "mp3",
  });

  test("uniform albums read like a single file", () => {
    expect(
      describeCollectionFormat([
        track("flac", 44_100, 24),
        track("flac", 44_100, 24),
      ]),
    ).toEqual({
      summary: "FLAC 44.1kHz 24bit",
      quality: "lossless",
      highRes: true,
    });
  });

  test("mixed albums show ranges and the worst quality", () => {
    expect(
      describeCollectionFormat([
        track("flac", 44_100, 16),
        track("flac", 192_000, 24),
        track("mp3", 48_000, null),
      ]),
    ).toEqual({
      summary: "FLAC / MP3 44.1\u2013192kHz 16\u201324bit",
      quality: "low_quality",
      highRes: true,
    });
    expect(describeCollectionFormat([null, undefined])).toBeNull();
    expect(
      describeCollectionFormat([track("flac", 44_100, 16), null])?.quality,
    ).toBeNull();
  });
});

describe("volume leveling", () => {
  const loudness = {
    track_gain_db: -6.4,
    track_peak: 0.98,
    album_gain_db: 3,
    album_peak: 0.5,
  };

  test("track and album modes pick their tag and fall back to the other", () => {
    expect(volumeLevelingGain(loudness, "track")).toEqual(
      expect.objectContaining({
        appliedDb: -6.4,
        source: "track",
        limitedBy: null,
      }),
    );
    expect(volumeLevelingGain(loudness, "album")?.source).toBe("album");
    expect(volumeLevelingGain({ album_gain_db: -2 }, "track")?.source).toBe(
      "album",
    );
    expect(volumeLevelingGain(loudness, "auto")?.source).toBe("track");
    expect(
      volumeLevelingGain(loudness, "auto", { albumContext: true })?.source,
    ).toBe("album");
    expect(volumeLevelingGain(loudness, "off")).toBeNull();
    expect(volumeLevelingGain({}, "track")).toBeNull();
  });

  test("positive gains stop at the peak and at full volume", () => {
    const peak = volumeLevelingGain(
      { track_gain_db: 9, track_peak: 0.5 },
      "track",
    );
    expect(peak?.limitedBy).toBe("peak");
    expect(peak?.appliedDb).toBeCloseTo(6.02, 2);
    const ceiling = volumeLevelingGain({ track_gain_db: 2 }, "track", {
      maxGainDb: 0,
    });
    expect(ceiling).toEqual(
      expect.objectContaining({
        appliedDb: 0,
        requestedDb: 2,
        limitedBy: "full_volume",
      }),
    );
    const preamp = volumeLevelingGain({ track_gain_db: -4 }, "track", {
      preampDb: 3,
    });
    expect(preamp?.appliedDb).toBe(-1);
  });

  test("leveling shows as an enhanced stage with its limit", () => {
    const leveled = buildSignalPath(originalServer(), {
      ...plainBrowser,
      leveling: volumeLevelingGain({ track_gain_db: 2 }, "track", {
        maxGainDb: 0,
      }),
    });
    const stage = leveled.stages.find((item) => item.id === "client-leveling");
    expect(stage?.detail).toContain("lower the volume");
    expect(stage?.quality).toBe("enhanced");
    const untagged = buildSignalPath(originalServer(), {
      ...plainBrowser,
      levelingUnavailable: true,
    });
    expect(
      untagged.stages.find((item) => item.id === "client-leveling")?.quality,
    ).toBeNull();
    expect(untagged.quality).toBe("high_quality");
  });
});

describe("signal path resource", () => {
  afterEach(() => {
    mock.restore();
    clearCachedSignalPaths();
  });

  test("requests the server leg with stream parameters and caches it", async () => {
    const get = spyOn(api, "get").mockResolvedValue({
      data: originalServer(),
    } as never);
    await getSongSignalPath("folder/song #1", { bitrate: 320, target: "cast" });
    await getSongSignalPath("folder/song #1", { bitrate: 320, target: "cast" });
    expect(get).toHaveBeenCalledTimes(1);
    expect(get).toHaveBeenCalledWith(
      "/media/songs/folder%2Fsong%20%231/signal-path",
      { params: { bitrate: 320, target: "cast" } },
    );
  });

  test("failed requests are not cached", async () => {
    const get = spyOn(api, "get").mockRejectedValue(new Error("offline"));
    await expect(getSongSignalPath("song")).rejects.toThrow("offline");
    await expect(getSongSignalPath("song")).rejects.toThrow("offline");
    expect(get).toHaveBeenCalledTimes(2);
  });
});
