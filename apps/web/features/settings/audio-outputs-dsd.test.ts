import { expect, test } from "bun:test";
import type { ZoneEndpoint } from "@parson/music-sdk";
import {
  descriptorLines,
  type OutputSetup,
  dsdStrategyNote,
  dsdSupportSummary,
  hardwareSummary,
  maxDsdRateOptions,
} from "./audio-outputs-model";

const dsdRates = [2_822_400, 5_644_800, 11_289_600, 22_579_200];

function setup(settings: Partial<OutputSetup> = {}): OutputSetup {
  return {
    exclusive: true,
    max_sample_rate: null,
    max_bit_depth: null,
    dsd_strategy: "auto",
    ...settings,
  };
}

function dac(native: boolean) {
  const device: ZoneEndpoint = {
    id: "device:alsa:D10s:0",
    name: "Topping D10s",
    detail: "hw:CARD=D10s,DEV=0",
    driver: "ALSA",
    mode_label: "ALSA hw, exclusive",
    address: "hw:CARD=D10s,DEV=0",
    exclusive_capable: true,
    probe_error: null,
    formats: {
      sample_rates: [],
      bit_depths: [],
      dsd_rates: dsdRates.map((value) => ({
        value,
        supported: native && value <= 11_289_600,
        allowed: true,
      })),
      dop_rates: [2_822_400, 5_644_800],
    },
    sample_formats: ["24-bit packed", "32-bit"],
    channels: [2, 2],
    network: null,
    dsp_support: null,
    volume: null,
    has_icon: false,
    dsd_formats: native ? ["DSD_U32_BE"] : [],
    hardware: {
      bus: "usb",
      usb_id: "152a:8750",
      product: "Topping D10s",
      speed: "high speed",
      caps_source: "usb_descriptors",
      altsets: [
        {
          interface: 1,
          altset: 1,
          formats: ["S32_LE", "DSD_U32_BE"],
          channels: 2,
          rates: [44_100, 384_000],
          bits: 32,
        },
      ],
    },
  };
  return device;
}

test("DSD support names native rates, the container and the DoP fallback", () => {
  expect(dsdSupportSummary(dac(true))).toBe("Native DSD64–DSD256 (DSD_U32_BE)");
  expect(dsdSupportSummary(dac(false))).toBe(
    "No native DSD; DoP carrier fits DSD64–DSD128",
  );
  expect(maxDsdRateOptions(dac(true)).map((option) => option.label)).toEqual([
    "DSD64",
    "DSD128",
    "DSD256",
  ]);
});

test("the strategy note says what will actually happen", () => {
  expect(dsdStrategyNote(dac(false), setup())).toContain("choose DoP");
  expect(dsdStrategyNote(dac(false), setup({ dsd_strategy: "dop" }))).toContain(
    "does not decode DoP",
  );
  expect(dsdStrategyNote(dac(true), setup({ exclusive: false }))).toContain(
    "needs Exclusive Mode",
  );
  expect(dsdStrategyNote(dac(true), setup())).toContain(
    "Native DSD when the driver",
  );
});

test("hardware details come from the kernel's USB report", () => {
  expect(hardwareSummary(dac(true))).toBe(
    "USB 152a:8750 · high speed · device busy: formats from its USB descriptors",
  );
  expect(descriptorLines(dac(true))).toEqual([
    "S32_LE DSD_U32_BE, 32 bit, 2ch, 44.1kHz–384kHz",
  ]);
});
