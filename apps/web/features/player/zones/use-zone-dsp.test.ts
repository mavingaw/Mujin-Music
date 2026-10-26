import { describe, expect, test } from "bun:test";
import { defaultDspSettings, type DspSettings } from "@parson/music-sdk";
import { initialServerDsp } from "./use-server-dsp";

function withEq(gainDb: number): DspSettings {
  const settings = defaultDspSettings();
  settings.eq.enabled = true;
  settings.eq.bands[0]!.gainDb = gainDb;
  return settings;
}

describe("server output DSP settings", () => {
  test("the server's saved chain wins", () => {
    const initial = initialServerDsp(
      { dsp: withEq(4), dsp_customized: true },
      withEq(-2),
    );
    expect(initial.settings.eq.bands[0]!.gainDb).toBe(4);
    expect(initial.push).toBeFalse();
  });

  test("a chain this browser kept is pushed to a server that has none", () => {
    const initial = initialServerDsp(
      { dsp: defaultDspSettings(), dsp_customized: false },
      withEq(-2),
    );
    expect(initial.settings.eq.bands[0]!.gainDb).toBe(-2);
    expect(initial.push).toBeTrue();
  });

  test("defaults on both sides stay put, and old servers are not written", () => {
    expect(
      initialServerDsp(
        { dsp: defaultDspSettings(), dsp_customized: false },
        defaultDspSettings(),
      ).push,
    ).toBeFalse();
    expect(initialServerDsp({}, withEq(3)).push).toBeFalse();
  });
});
