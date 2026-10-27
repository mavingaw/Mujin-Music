import { describe, expect, test } from "bun:test";
import type { AudioProperties } from "@parson/music-sdk";

import {
  nativePlaybackCapabilities,
  offlineMediaPath,
  playsNatively,
  sourceFormatToken,
} from "./playback-formats";

const audio = (codec: string, sample_rate = 44_100): AudioProperties => ({
  codec,
  container: null,
  sample_rate,
  bit_depth: 24,
  channels: 2,
  bitrate_kbps: null,
  lossless: true,
});

describe("mobile playback formats", () => {
  test("each platform declares what its decoders play", () => {
    expect(nativePlaybackCapabilities("ios").accept).toContain("alac");
    expect(nativePlaybackCapabilities("android").accept).not.toContain("alac");
    expect(nativePlaybackCapabilities("android").accept).toContain("opus");
    expect(nativePlaybackCapabilities("web").maxSampleRate).toBe(192_000);
  });

  test("MP4 files are identified by codec", () => {
    expect(sourceFormatToken("/a/b.m4a", audio("alac"))).toBe("alac");
    expect(sourceFormatToken("/a/b.m4a", audio("aac"))).toBe("aac");
    expect(sourceFormatToken("/a/b.m4a", null)).toBeNull();
    expect(sourceFormatToken("/a/b.dsf", audio("dsd"))).toBeNull();
    expect(sourceFormatToken("/a/b.ogg", audio("opus"))).toBe("opus");
  });

  test("offline copies of unplayable formats are lossless FLAC", () => {
    const android = nativePlaybackCapabilities("android");
    const ios = nativePlaybackCapabilities("ios");
    expect(offlineMediaPath("/m/a.dsf", audio("dsd", 2_822_400), android)).toBe(
      "/m/a.dsf.flac",
    );
    expect(offlineMediaPath("/m/a.wv", audio("wavpack"), ios)).toBe(
      "/m/a.wv.flac",
    );
    expect(offlineMediaPath("/m/a.m4a", audio("alac"), android)).toBe(
      "/m/a.m4a.flac",
    );
    expect(offlineMediaPath("/m/a.m4a", audio("alac"), ios)).toBe("/m/a.m4a");
    expect(playsNatively("/m/a.flac", audio("flac", 384_000), ios)).toBeFalse();
    expect(playsNatively("/m/a.flac", audio("flac", 96_000), ios)).toBeTrue();
  });
});
