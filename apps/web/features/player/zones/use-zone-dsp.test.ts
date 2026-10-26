import { describe, expect, test } from "bun:test";
import {
  defaultDspSettings,
  type DspSettings,
  type Zone,
} from "@parson/music-sdk";
import { initialZoneDsp, zoneDspSupport } from "./use-zone-dsp";

function withEq(gainDb: number): DspSettings {
  const settings = defaultDspSettings();
  settings.eq.enabled = true;
  settings.eq.bands[0]!.gainDb = gainDb;
  return settings;
}

describe("a zone's DSP chain", () => {
  test("the zone's saved chain wins", () => {
    const initial = initialZoneDsp(withEq(4), withEq(-2));
    expect(initial.settings.eq.bands[0]!.gainDb).toBe(4);
    expect(initial.push).toBeFalse();
  });

  test("a chain this browser kept is copied into a zone that has none", () => {
    const initial = initialZoneDsp(defaultDspSettings(), withEq(-2));
    expect(initial.settings.eq.bands[0]!.gainDb).toBe(-2);
    expect(initial.push).toBeTrue();
  });

  test("defaults on both sides stay put, and zones without a chain are not written", () => {
    expect(
      initialZoneDsp(defaultDspSettings(), defaultDspSettings()).push,
    ).toBeFalse();
    expect(initialZoneDsp(null, withEq(3)).push).toBeFalse();
  });

  test("the engine's support comes from the zone's own endpoint", () => {
    const support = {
      available: false,
      reason: "Parson could not read the formats USB DAC accepts.",
      engine: "Parson DSP, 64-bit float",
      crossfeed: false,
      sample_rates: [],
      output_bits: null,
    };
    const device = {
      id: "device:alsa:USB:0",
      name: "USB DAC",
      endpoints: [{ id: "device:alsa:USB:0", dsp_support: support }],
      settings: { supports: { dsp: true } },
    } as unknown as Zone;
    expect(zoneDspSupport(device)).toEqual(support);
    // A served stream is rendered at the track's rate: no fixed rates.
    const served = {
      ...device,
      endpoints: [],
    } as unknown as Zone;
    expect(zoneDspSupport(served)?.available).toBeTrue();
    expect(zoneDspSupport(served)?.sample_rates).toEqual([]);
    const app = {
      ...served,
      settings: { supports: { dsp: false } },
    } as unknown as Zone;
    expect(zoneDspSupport(app)).toBeNull();
  });
});
