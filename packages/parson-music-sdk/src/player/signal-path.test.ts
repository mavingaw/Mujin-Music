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
  test("an untouched stream through a shared-mode output is high quality, not lossless", () => {
    const path = buildSignalPath(originalServer(), {
      ...plainBrowser,
      deviceSampleRate: 96_000,
      output: { kind: "system", name: "USB DAC", chosen: true },
    });
    expect(ids(path.stages)).toEqual([
      "source",
      "server",
      "client-decode",
      "client-output",
    ]);
    // Roon rates an OS mixer output high quality: the browser cannot open
    // the device exclusively, so the system may resample or mix.
    expect(path.quality).toBe("high_quality");
    expect(path.bitPerfectToOutput).toBeTrue();
    expect(path.bitPerfect).toBeFalse();
    expect(path.highRes).toBeTrue();
    expect(path.stages[3]).toEqual(
      expect.objectContaining({
        title: "USB DAC",
        detail: "Selected output, shared mode, 96kHz",
        quality: "high_quality",
      }),
    );
    expect(path.stages[3]!.explanation).toContain("server output");
    expect(path.description).toBe(
      "USB DAC is reached in shared mode, where the system can resample the audio and mix in other sounds, so Parson cannot promise it arrives unmodified.",
    );
    expect(path.description).not.toContain("without being modified");
    expect(signalPathSummary(path)).toBe("FLAC 96kHz 24bit 2ch");
  });

  test("a device at another rate adds the browser's resampling", () => {
    const path = buildSignalPath(originalServer(), {
      ...plainBrowser,
      deviceSampleRate: 48_000,
    });
    expect(ids(path.stages)).toEqual([
      "source",
      "server",
      "client-decode",
      "client-device-resample",
      "client-output",
    ]);
    expect(path.stages[3]!.detail).toBe("96kHz to 48kHz");
    expect(path.stages[4]!.title).toBe("System default output");
    expect(path.quality).toBe("high_quality");
    expect(path.description).toBe(
      "The audio is changed in ways you did not ask for, with very little loss: sample rate conversion 96kHz to 48kHz. System default output is reached in shared mode, where the system can resample the audio and mix in other sounds, so Parson cannot promise it arrives unmodified.",
    );
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
      "client-depth-in",
      "client-speed",
      "client-resample",
      "client-reverb",
      "client-depth-out",
      "client-output",
    ]);
    expect(presets.stages[3]!.detail).toBe("24bit to 32bit float");
    expect(presets.stages[4]!.detail).toBe("0.78x, pitch follows speed");
    expect(presets.stages[5]!.detail).toBe("96kHz to 48kHz");
    expect(presets.stages[6]!.detail).toBe("Convolution, 1.8s room, 18% wet");
    expect(presets.stages[7]!.detail).toBe("32bit float to output format");
    // The unrequested resample decides the light, and the summary says
    // both what was and was not asked for.
    expect(presets.quality).toBe("high_quality");
    expect(presets.description).toContain("you did not ask for");
    expect(presets.description).toContain(
      "It also applies processing you asked for",
    );
    expect(presets.bitPerfectToOutput).toBeFalse();

    const matched = buildSignalPath(originalServer(), {
      ...plainBrowser,
      engine: "web-audio",
      contextSampleRate: 96_000,
      playbackRate: 0.78,
      preservePitch: false,
      reverbWet: 0.18,
      presetLabel: "Slow",
    });
    // Processing you asked for is enhanced, but the shared-mode output
    // still decides the light.
    expect(
      matched.stages.find((stage) => stage.id === "client-speed")?.quality,
    ).toBe("enhanced");
    expect(matched.quality).toBe("high_quality");
    expect(matched.description).toContain("reached in shared mode");
    expect(matched.description).toContain(
      "It also applies processing you asked for: Slow: speed",
    );
    expect(matched.description).not.toContain("did not ask");

    const quiet = buildSignalPath(originalServer(), {
      ...plainBrowser,
      volume: 0.5,
    });
    expect(quiet.stages.find((stage) => stage.id === "client-volume")).toEqual(
      expect.objectContaining({ detail: "−6.0 dB", quality: "high_quality" }),
    );
  });

  test("a server output in exclusive mode is bit-perfect and the client adds nothing", () => {
    const server: ServerSignalPath = {
      ...originalServer(),
      target: "native",
      mode: "native",
      stages: [
        originalServer().stages[0]!,
        {
          id: "native-decode",
          kind: "decode",
          title: "Parson server",
          detail: "FLAC decoded to PCM",
          quality: "lossless",
          explanation: "",
        },
        {
          id: "native-output",
          kind: "output",
          title: "USB DAC",
          detail: "WASAPI Exclusive Mode, 96kHz 24bit 2ch",
          quality: "lossless",
          explanation: "",
        },
      ],
    };
    const path = buildSignalPath(server, {
      ...plainBrowser,
      volume: 0.3,
      engine: "web-audio",
      contextSampleRate: 48_000,
      deviceSampleRate: 44_100,
      output: { kind: "native", name: "USB DAC" },
    });
    expect(ids(path.stages)).toEqual([
      "source",
      "native-decode",
      "native-output",
    ]);
    expect(path.quality).toBe("lossless");
    expect(path.bitPerfect).toBeTrue();
    expect(path.highRes).toBeTrue();
    expect(path.description).toBe(
      "A high-resolution lossless stream reaches USB DAC bit-perfect, in exclusive mode at 96kHz 24bit 2ch.",
    );
    // A forced conversion on the server output is high quality.
    const converted = buildSignalPath(
      {
        ...server,
        stages: [
          ...server.stages.slice(0, 2),
          {
            id: "native-resample",
            kind: "conversion",
            title: "Sample rate conversion",
            detail: "192kHz to 96kHz (soxr, very high quality)",
            quality: "high_quality",
            explanation: "",
          },
          server.stages[2]!,
        ],
      },
      { ...plainBrowser, output: { kind: "native" } },
    );
    expect(converted.quality).toBe("high_quality");
    expect(converted.bitPerfect).toBeFalse();
    expect(converted.description).toBe(
      "The audio is changed in ways you did not ask for, with very little loss: sample rate conversion 192kHz to 96kHz (soxr, very high quality).",
    );
  });

  test("a hi-res source transcoded to a lossy format is not badged hi-res", () => {
    const server = originalServer();
    server.mode = "transcode";
    server.stages.splice(1, 0, {
      id: "encode",
      kind: "encode",
      title: "Transcode",
      detail: "MP3 320kbps (LAME)",
      quality: "low_quality",
      explanation: "",
    });
    const path = buildSignalPath(server, plainBrowser);
    expect(path.source?.high_res).toBeTrue();
    expect(path.quality).toBe("low_quality");
    expect(path.highRes).toBeFalse();
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
    expect(path.stages.every((stage) => stage.quality === "low_quality")).toBe(
      true,
    );
    expect(path.description).toBe(
      "A lossy format is involved: the source is a lossy file (MP3 44.1kHz 2ch 320kbps).",
    );
  });

  test("stages after a lossy transcode are low quality, not lossless", () => {
    const server = originalServer();
    server.mode = "transcode";
    server.stages.splice(1, 0, {
      id: "encode",
      kind: "encode",
      title: "Transcode",
      detail: "MP3 320kbps (LAME)",
      quality: "low_quality",
      explanation: "",
    });
    server.delivered = {
      ...hiRes,
      codec: "mp3",
      lossless: false,
      bit_depth: null,
      summary: "MP3 48kHz 2ch 320kbps",
    };
    const path = buildSignalPath(server, {
      ...plainBrowser,
      leveling: volumeLevelingGain({ track_gain_db: -4 }, "track"),
    });
    expect(path.stages.map((stage) => [stage.id, stage.quality])).toEqual([
      ["source", "lossless"],
      ["encode", "low_quality"],
      ["server", "low_quality"],
      ["client-decode", "low_quality"],
      ["client-leveling", "low_quality"],
      ["client-output", "low_quality"],
    ]);
    expect(path.description).toBe(
      "A lossy format is involved: transcode to MP3 320kbps (LAME). Parson also applies processing you asked for: Volume leveling (−4.0 dB, track ReplayGain).",
    );
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
    // Leveling that cannot run leaves the samples alone; the shared-mode
    // output is what keeps both paths at high quality.
    expect(
      untagged.stages.filter((item) => item.quality === "high_quality"),
    ).toEqual([expect.objectContaining({ id: "client-output" })]);
    expect(untagged.quality).toBe("high_quality");
    expect(leveled.quality).toBe("high_quality");
    expect(leveled.description).toContain(
      "It also applies processing you asked for: Volume leveling",
    );
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
