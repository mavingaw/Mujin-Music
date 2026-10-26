import { describe, expect, test } from "bun:test";
import type { AudioProperties } from "@parson/music-sdk/types";
import {
  albumAudioSummary,
  audioBadgeDescription,
  audioBadgeLabel,
} from "./audio-quality";

const audio = (patch: Partial<AudioProperties>): AudioProperties => ({
  codec: "flac",
  container: "flac",
  sample_rate: 44_100,
  bit_depth: 16,
  channels: 2,
  bitrate_kbps: 900,
  lossless: true,
  ...patch,
});

describe("album audio summaries", () => {
  test("a consistent album shows its format", () => {
    const summary = albumAudioSummary([
      { audio: audio({ sample_rate: 96_000, bit_depth: 24 }) },
      { audio: audio({ sample_rate: 96_000, bit_depth: 24 }) },
    ]);
    expect(summary && audioBadgeLabel(summary)).toBe("FLAC 24/96");
    expect(summary?.mixed).toBeFalse();
    expect(summary && audioBadgeDescription(summary)).toBe(
      "High-resolution lossless · FLAC · 24-bit / 96 kHz · stereo",
    );
  });

  test("mixed albums show the best copy and say they are mixed", () => {
    const summary = albumAudioSummary([
      { audio: audio({ codec: "mp3", lossless: false, bitrate_kbps: 320 }) },
      {
        audio: audio({
          codec: "dsd",
          container: "dsf",
          sample_rate: 2_822_400,
          bit_depth: 1,
        }),
      },
      { audio: null },
    ]);
    expect(summary && audioBadgeLabel(summary)).toBe("DSD64");
    expect(summary?.mixed).toBeTrue();
    expect(albumAudioSummary([{ audio: null }])).toBeNull();
  });
});
