import { describe, expect, test } from "bun:test";
import {
  applyPlaybackCapabilities,
  detectPlaybackCapabilities,
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
