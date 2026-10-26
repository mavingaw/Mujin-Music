import { describe, expect, test } from "bun:test";
import {
  applyPlaybackCapabilities,
  describeAudioFormat,
  detectPlaybackCapabilities,
  dsdRateName,
  playbackCapabilityParams,
} from "./formats";

const chrome: Record<string, string> = {
  "audio/flac": "probably",
  'audio/mp4; codecs="mp4a.40.2"': "probably",
  "audio/mpeg": "probably",
  'audio/ogg; codecs="vorbis"': "probably",
  'audio/ogg; codecs="opus"': "probably",
  'audio/wav; codecs="1"': "probably",
  'audio/mp4; codecs="alac"': "",
  "audio/aiff": "",
};

describe("playback capabilities", () => {
  test("detects what a media element decodes", () => {
    const capabilities = detectPlaybackCapabilities(
      (mime) => chrome[mime] ?? "",
      { maxSampleRate: 192_000 },
    );
    expect(capabilities.accept).toEqual([
      "flac",
      "aac",
      "mp3",
      "vorbis",
      "opus",
      "wav",
    ]);
    expect(capabilities.maxSampleRate).toBe(192_000);
  });

  test("codec probes require a definite answer", () => {
    const capabilities = detectPlaybackCapabilities((mime) =>
      mime.includes("codecs=") ? "maybe" : "",
    );
    expect(capabilities.accept).toEqual([]);
    expect(
      detectPlaybackCapabilities(() => {
        throw new Error("no media element");
      }).accept,
    ).toEqual([]);
  });

  test("capabilities become stream query parameters", () => {
    const url = applyPlaybackCapabilities(
      new URL("https://music.example/api/v1/media/songs/a/stream"),
      {
        accept: ["flac", "mp3"],
        maxSampleRate: 96_000,
        maxChannels: 2,
        dsd: "dop",
      },
    );
    expect(url.searchParams.get("accept")).toBe("flac,mp3");
    expect(url.searchParams.get("max_sample_rate")).toBe("96000");
    expect(url.searchParams.get("max_channels")).toBe("2");
    expect(url.searchParams.get("dsd")).toBe("dop");
    expect(playbackCapabilityParams({ accept: [], dsd: "dop" })).toEqual({});
  });
});

describe("audio format summaries", () => {
  test("hi-res, CD-quality, DSD and lossy tracks get distinct badges", () => {
    expect(
      describeAudioFormat({
        codec: "flac",
        container: "flac",
        sample_rate: 96_000,
        bit_depth: 24,
        channels: 2,
        bitrate_kbps: 3_100,
        lossless: true,
      }),
    ).toEqual({
      name: "FLAC",
      resolution: "24-bit / 96 kHz",
      badge: "24/96",
      channels: "stereo",
      tier: "hi_res",
    });
    expect(
      describeAudioFormat({
        codec: "ape",
        container: "ape",
        sample_rate: 44_100,
        bit_depth: 16,
        channels: 2,
        bitrate_kbps: null,
        lossless: true,
      })?.badge,
    ).toBe("16/44.1");
    const dsd = describeAudioFormat({
      codec: "dsd",
      container: "dsf",
      sample_rate: 5_644_800,
      bit_depth: 1,
      channels: 6,
      bitrate_kbps: null,
      lossless: true,
    });
    expect(dsd?.badge).toBe("DSD128");
    expect(dsd?.tier).toBe("dsd");
    expect(dsd?.channels).toBe("5.1");
    const mp3 = describeAudioFormat({
      codec: "mp3",
      container: "mp3",
      sample_rate: 44_100,
      bit_depth: null,
      channels: 2,
      bitrate_kbps: 320,
      lossless: false,
    });
    expect(mp3?.tier).toBe("lossy");
    expect(mp3?.badge).toBe("320k");
    expect(
      describeAudioFormat({
        codec: "pcm",
        container: "aiff",
        sample_rate: 44_100,
        bit_depth: 16,
        channels: 2,
        bitrate_kbps: null,
        lossless: true,
      })?.name,
    ).toBe("AIFF");
    expect(describeAudioFormat(null)).toBeNull();
  });

  test("DSD rates are named by multiples of 44.1 or 48 kHz", () => {
    expect(dsdRateName(2_822_400)).toBe("DSD64");
    expect(dsdRateName(11_289_600)).toBe("DSD256");
    expect(dsdRateName(3_072_000)).toBe("DSD64");
    expect(dsdRateName(96_000)).toBeNull();
  });
});
