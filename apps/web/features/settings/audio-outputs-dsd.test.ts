import { expect, test } from "bun:test";
import type { NativeOutputDevice } from "@parson/music-sdk";
import {
  descriptorLines,
  dsdStrategyNote,
  dsdSupportSummary,
  hardwareSummary,
  maxDsdRateOptions,
} from "./audio-outputs-model";

const dsdRates = [2_822_400, 5_644_800, 11_289_600, 22_579_200];

function dac(
  native: boolean,
  settings: Partial<NativeOutputDevice["settings"]> = {},
) {
  const device: NativeOutputDevice = {
    id: "alsa:D10s:0",
    name: "Topping D10s",
    detail: "hw:CARD=D10s,DEV=0",
    driver: "alsa",
    mode_label: "ALSA hw, exclusive",
    exclusive_capable: true,
    is_default: false,
    probe_error: null,
    settings: {
      exclusive: true,
      max_sample_rate: null,
      max_bit_depth: null,
      dsd_strategy: "auto",
      max_dsd_rate: null,
      ...settings,
    },
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
    in_use: false,
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
  expect(dsdStrategyNote(dac(false))).toContain("choose DoP");
  expect(dsdStrategyNote(dac(false, { dsd_strategy: "dop" }))).toContain(
    "does not decode DoP",
  );
  expect(dsdStrategyNote(dac(true, { exclusive: false }))).toContain(
    "needs Exclusive Mode",
  );
  expect(dsdStrategyNote(dac(true))).toContain("Native DSD when the driver");
});

test("hardware details come from the kernel's USB report", () => {
  expect(hardwareSummary(dac(true))).toBe(
    "USB 152a:8750 · high speed · device busy: formats from its USB descriptors",
  );
  expect(descriptorLines(dac(true))).toEqual([
    "S32_LE DSD_U32_BE, 32 bit, 2ch, 44.1kHz–384kHz",
  ]);
});
