import { describe, expect, test } from "bun:test";
import type { NativeOutputDevice } from "@parson/music-sdk";
import { isDspShortcut, panelAvailability } from "./dsp-panel";

const key = (overrides: Partial<KeyboardEvent>) => ({
  code: "Slash",
  key: "/",
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...overrides,
});

describe("DSP engine shortcut", () => {
  test("Ctrl+/ and Cmd+/ open it, like Roon's MUSE", () => {
    expect(isDspShortcut(key({ ctrlKey: true }))).toBeTrue();
    expect(isDspShortcut(key({ metaKey: true }))).toBeTrue();
    // Keyboard layouts that put "/" elsewhere still match by key.
    expect(
      isDspShortcut(key({ ctrlKey: true, code: "Digit7", key: "/" })),
    ).toBeTrue();
  });

  test("plain or shifted slashes do not", () => {
    expect(isDspShortcut(key({}))).toBeFalse();
    expect(isDspShortcut(key({ ctrlKey: true, shiftKey: true }))).toBeFalse();
    expect(isDspShortcut(key({ ctrlKey: true, altKey: true }))).toBeFalse();
    expect(
      isDspShortcut(key({ ctrlKey: true, code: "KeyK", key: "k" })),
    ).toBeFalse();
  });
});

const dac = {
  id: "alsa:USB:0",
  name: "USB DAC",
  dsp_support: {
    available: true,
    reason: null,
    engine: "Parson DSP, 64-bit float",
    crossfeed: true,
    sample_rates: [44_100, 48_000, 88_200, 96_000],
    output_bits: 32,
  },
} as unknown as NativeOutputDevice;

describe("where the DSP engine runs", () => {
  test("a server output runs the chain on the server in 64-bit float", () => {
    const availability = panelAvailability(
      { serverOutputId: dac.id, serverDevices: [dac] },
      false,
      false,
    );
    expect(availability.available).toBeTrue();
    expect(availability.precision).toBe("64-bit float");
    expect(availability.sampleRates).toEqual([44_100, 48_000, 88_200, 96_000]);
  });

  test("controls grey out only when the chain truly cannot apply", () => {
    expect(
      panelAvailability({ serverOutputId: null, serverDevices: [] }, true)
        .available,
    ).toBeFalse();
    expect(
      panelAvailability(
        { serverOutputId: null, serverDevices: [] },
        false,
        false,
      ).available,
    ).toBeFalse();
    const unprobed = {
      ...dac,
      dsp_support: {
        ...dac.dsp_support!,
        available: false,
        reason: "Parson could not read the formats USB DAC accepts.",
      },
    };
    const blocked = panelAvailability(
      { serverOutputId: dac.id, serverDevices: [unprobed] },
      false,
    );
    expect(blocked.available).toBeFalse();
    expect(blocked.reason).toContain("USB DAC");
    expect(
      panelAvailability(
        { serverOutputId: null, serverDevices: [] },
        false,
        true,
      ).available,
    ).toBeTrue();
  });
});
