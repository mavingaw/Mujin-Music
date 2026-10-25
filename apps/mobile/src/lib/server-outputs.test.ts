import { expect, test } from "bun:test";
import type {
  NativeOutputDevice,
  NativeOutputSession,
} from "@parson/music-sdk";
import {
  serverOutputSummary,
  serverSessionLine,
  unsupportedRates,
} from "./server-outputs";

const device = {
  id: "wasapi:{1}",
  name: "USB DAC",
  formats: {
    sample_rates: [44_100, 96_000, 192_000, 352_800].map((value) => ({
      value,
      supported: value <= 192_000,
      allowed: true,
    })),
    bit_depths: [16, 24, 32].map((value) => ({
      value,
      supported: value < 32,
      allowed: true,
    })),
  },
  probe_error: null,
} as unknown as NativeOutputDevice;

test("summaries list what the device plays bit-perfectly", () => {
  expect(serverOutputSummary(device)).toBe(
    "Bit-perfect 44.1kHz–192kHz, 16/24 bit",
  );
  expect(unsupportedRates(device)).toBe("352.8kHz");
  expect(
    serverOutputSummary({
      ...device,
      formats: null,
      probe_error: "in use",
    }),
  ).toBe("in use");
});

test("the session line shows state and position", () => {
  expect(
    serverSessionLine({
      state: "paused",
      position_ms: 83_400,
      duration_ms: 240_000,
    } as NativeOutputSession),
  ).toBe("Paused · 1:23 / 4:00");
  expect(
    serverSessionLine({
      state: "failed",
      error: "device busy",
      position_ms: 0,
      duration_ms: 0,
    } as NativeOutputSession),
  ).toBe("device busy · 0:00 / 0:00");
});
