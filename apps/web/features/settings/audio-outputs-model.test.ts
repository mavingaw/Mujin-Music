import { expect, test } from "bun:test";
import type { ZoneEndpoint } from "@parson/music-sdk";
import {
  deviceCapabilitySummary,
  formatTone,
  hardwareSummary,
  maxDepthOptions,
  maxRateOptions,
} from "./audio-outputs-model";

const rates = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800];

const device: ZoneEndpoint = {
  id: "device:alsa:USB:0",
  name: "USB DAC",
  detail: "hw:CARD=USB,DEV=0",
  driver: "ALSA",
  mode_label: "ALSA hw, exclusive",
  address: "hw:CARD=USB,DEV=0",
  exclusive_capable: true,
  probe_error: null,
  formats: {
    sample_rates: rates.map((value) => ({
      value,
      supported: value <= 192_000,
      allowed: value <= 96_000,
    })),
    bit_depths: [16, 24, 32].map((value) => ({
      value,
      supported: value !== 32,
      allowed: true,
    })),
  },
  sample_formats: ["16-bit", "24-bit packed"],
  channels: [2, 2],
  dsd_formats: [],
  hardware: null,
  network: null,
  dsp_support: null,
  volume: null,
  has_icon: false,
};

test("unsupported rates are red even when above the limit", () => {
  expect(formatTone({ value: 44_100, supported: true, allowed: true })).toBe(
    "supported",
  );
  expect(formatTone({ value: 192_000, supported: true, allowed: false })).toBe(
    "capped",
  );
  expect(formatTone({ value: 352_800, supported: false, allowed: false })).toBe(
    "unsupported",
  );
});

test("limits offer only what the device supports", () => {
  expect(maxRateOptions(device).map((option) => option.label)).toEqual([
    "44.1kHz",
    "48kHz",
    "88.2kHz",
    "96kHz",
    "176.4kHz",
    "192kHz",
  ]);
  expect(maxDepthOptions(device).map((option) => option.value)).toEqual([
    16, 24,
  ]);
});

test("the capability summary reads like a spec line", () => {
  expect(deviceCapabilitySummary(device)).toBe(
    "44.1kHz to 192kHz, 16/24 bit, 2 channels",
  );
  expect(
    deviceCapabilitySummary({
      ...device,
      formats: null,
      probe_error: "hw:1,0: the device is in use by another program",
    }),
  ).toContain("in use");
});

test("an emulated card says it has no DAC behind it", () => {
  const hardware = {
    bus: "usb",
    usb_id: "1209:0001",
    product: "Parson Bench DAC (kernel ABI emulator)",
    speed: "high speed",
    caps_source: "driver" as const,
    altsets: [],
  };
  expect(hardwareSummary({ ...device, hardware })).toBe(
    "USB 1209:0001 · high speed · formats probed by the driver",
  );
  expect(
    hardwareSummary({
      ...device,
      hardware: { ...hardware, kernel_emulator: true },
    }),
  ).toBe(
    "USB 1209:0001 · high speed · formats probed by the driver · kernel ABI emulator, no DAC hardware attached",
  );
});
