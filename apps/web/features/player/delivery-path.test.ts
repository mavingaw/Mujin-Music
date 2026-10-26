import { describe, expect, test } from "bun:test";
import type { DeliveryPlan } from "@parson/music-sdk";
import {
  deliveryPathRows,
  pathQuality,
  pathQualityLabel,
} from "./delivery-path";

const format = {
  codec: "flac",
  container: "flac",
  sample_rate: 176_400,
  bit_depth: 24,
  channels: 2,
  bitrate_kbps: null,
  lossless: true,
};

const dsdPlan: DeliveryPlan = {
  mode: "lossless_conversion",
  content_type: "audio/flac",
  source: { ...format, name: "DSD64", codec: "dsd", bit_depth: 1 },
  output: { ...format, name: "FLAC" },
  steps: [
    {
      kind: "source",
      description: "DSD64, 2.8224 MHz, stereo",
      quality: "lossless",
    },
    { kind: "decode", description: "Decode DSD64", quality: "lossless" },
    {
      kind: "dsd_to_pcm",
      description: "Convert DSD64 to 24-bit / 176.4 kHz PCM (SoX VHQ)",
      quality: "high",
    },
    {
      kind: "encode",
      description: "Encode lossless FLAC 24-bit / 176.4 kHz, stereo",
      quality: "lossless",
    },
  ],
  quality: "high_quality",
  bit_perfect: false,
  gapless: true,
  reason:
    "This player cannot decode DSD, so it is converted to high-resolution PCM.",
};

describe("signal path rows", () => {
  test("a DSD conversion is shown step by step and lights high quality", () => {
    const rows = deliveryPathRows(dsdPlan);
    expect(rows.map((row) => row.title)).toEqual([
      "Source",
      "Decode",
      "DSD to PCM",
      "Encode",
      "This device",
    ]);
    expect(rows.at(-1)?.detail).toBe("Decodes FLAC without loss");
    expect(pathQualityLabel(pathQuality(rows))).toBe("High quality");
  });

  test("an original stream is bit-perfect until a preset changes the sound", () => {
    const original: DeliveryPlan = {
      ...dsdPlan,
      mode: "original",
      source: { ...format, name: "FLAC" },
      steps: [
        {
          kind: "source",
          description: "FLAC, 24-bit / 96 kHz",
          quality: "lossless",
        },
        {
          kind: "passthrough",
          description: "Original file bytes, untouched",
          quality: "bit_perfect",
        },
      ],
      quality: "bit_perfect",
      bit_perfect: true,
    };
    expect(pathQuality(deliveryPathRows(original))).toBe("lossless");
    const enhanced = deliveryPathRows(original, { presetLabel: "Deep reverb" });
    expect(pathQuality(enhanced)).toBe("enhanced");
    expect(deliveryPathRows(original, { casting: true }).at(-1)?.title).toBe(
      "Cast receiver",
    );
    expect(deliveryPathRows(null)).toEqual([]);
  });
});
