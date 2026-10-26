import { expect, test } from "bun:test";
import type { NativeOutputDevice } from "@parson/music-sdk";
import { outputCapabilityLine } from "./output-picker";

const formats = (max: number, depths: number[]) => ({
  sample_rates: [44_100, 48_000, 96_000, 192_000].map((value) => ({
    value,
    supported: value <= max,
    allowed: true,
  })),
  bit_depths: [16, 24, 32].map((value) => ({
    value,
    supported: depths.includes(value),
    allowed: true,
  })),
});

test("the picker says what each output can reach", () => {
  const dac = {
    formats: formats(192_000, [16, 24]),
    settings: { exclusive: true },
    probe_error: null,
  } as unknown as NativeOutputDevice;
  expect(outputCapabilityLine(dac)).toEqual({
    text: "Bit-perfect up to 192kHz/24bit, exclusive",
    tone: "lossless",
  });
  const streamer = {
    ...dac,
    network: { protocol: "open_home", caps_source: "assumed" },
  } as unknown as NativeOutputDevice;
  expect(outputCapabilityLine(streamer).text).toBe(
    "Bit-perfect up to 192kHz/24bit (assumed), DSP on the server",
  );
  const airplay = {
    ...dac,
    formats: formats(44_100, [16]),
    network: { protocol: "air_play", caps_source: "protocol" },
  } as unknown as NativeOutputDevice;
  expect(outputCapabilityLine(airplay)).toEqual({
    text: "High quality: AirPlay carries 44.1kHz/16bit, the receiver sets the volume",
    tone: "high_quality",
  });
  const unreachable = {
    formats: null,
    probe_error: "Parson could not reach this renderer.",
  } as unknown as NativeOutputDevice;
  expect(outputCapabilityLine(unreachable).tone).toBe("unknown");
});
