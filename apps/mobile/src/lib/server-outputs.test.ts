import { expect, test } from "bun:test";
import {
  defaultDspSettings,
  type NativeOutputDevice,
  type NativeOutputSession,
} from "@parson/music-sdk";
import {
  serverDspStatusLine,
  serverDspSummary,
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

test("the server's DSP chain is summarised per device", () => {
  expect(serverDspSummary(device)).toBeNull();
  const dsp = defaultDspSettings();
  expect(serverDspSummary({ ...device, dsp })).toBeNull();
  dsp.headroom = { enabled: true, gainDb: -3 };
  dsp.eq.enabled = true;
  dsp.eq.bands[0]!.gainDb = 4;
  dsp.eq.bands[3]!.gainDb = -2;
  dsp.crossfeed.enabled = true;
  dsp.sampleRate = { enabled: true, mode: 96_000 };
  expect(serverDspSummary({ ...device, dsp })).toBe(
    "Headroom −3.0 dB · EQ 2 bands · Crossfeed Medium · 96kHz",
  );
  expect(
    serverDspSummary({ ...device, dsp: { ...dsp, enabled: false } }),
  ).toBeNull();
});

test("the live DSP status reads like Roon's processing speed", () => {
  expect(serverDspStatusLine(null)).toBeNull();
  expect(
    serverDspStatusLine({
      realtime_factor: 41.27,
      peak_dbfs: 1.4,
      clipped_samples: 12,
      clipping: true,
    }),
  ).toBe("Processing speed 41.3x · Peak +1.4 dBFS · Clipping (12)");
});
