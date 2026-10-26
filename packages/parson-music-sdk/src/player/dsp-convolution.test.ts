import { describe, expect, test } from "bun:test";
import {
  activeConvolutionFilter,
  createDspChain,
  defaultDspSettings,
  dspAvailability,
  dspProcesses,
  normalizeDspSettings,
  type DspAudioContext,
} from "./dsp";

function fakeContext(convolver = true) {
  const created: { kind: string; buffer?: unknown; normalize?: boolean }[] = [];
  const node = (kind: string, extra: object = {}) => {
    const self = {
      kind,
      ...extra,
      connect: (to: unknown) => to,
      disconnect: () => {},
    };
    created.push(self);
    return self;
  };
  const context = {
    sampleRate: 48_000,
    createGain: () => node("gain", { gain: { value: 1 } }),
    createBiquadFilter: () =>
      node("biquad", {
        frequency: { value: 0 },
        Q: { value: 0 },
        gain: { value: 0 },
      }),
    createDelay: () => node("delay", { delayTime: { value: 0 } }),
    ...(convolver
      ? {
          createConvolver: () =>
            node("convolver", { buffer: null, normalize: true }),
        }
      : {}),
  };
  return { context: context as unknown as DspAudioContext, created };
}

describe("convolution", () => {
  test("settings keep a valid filter id and drop anything else", () => {
    const settings = normalizeDspSettings({
      convolution: { enabled: true, filterId: "0a1b2c3d4e5f6a7b" },
    });
    expect(settings.convolution).toEqual({
      enabled: true,
      filterId: "0a1b2c3d4e5f6a7b",
    });
    expect(activeConvolutionFilter(settings)).toBe("0a1b2c3d4e5f6a7b");
    expect(dspProcesses(settings)).toBeTrue();
    expect(
      normalizeDspSettings({
        convolution: { enabled: true, filterId: "../../etc/passwd" },
      }).convolution.filterId,
    ).toBeNull();
    // Older settings without the section stay bypassed.
    expect(normalizeDspSettings({}).convolution).toEqual({
      enabled: false,
      filterId: null,
    });
    const bypassed = { ...settings, enabled: false };
    expect(activeConvolutionFilter(bypassed)).toBeNull();
  });

  test("the browser chain adds an unnormalised convolver once the filter is loaded", () => {
    const { context, created } = fakeContext();
    const settings = defaultDspSettings();
    settings.convolution = { enabled: true, filterId: "abc123" };
    const impulse = { duration: 0.1 };
    let loaded: unknown = null;
    const chain = createDspChain(context, settings, {
      impulse: (id) => (id === "abc123" ? loaded : null),
    });
    expect(chain.describe()).toEqual([]);
    loaded = impulse;
    chain.update(settings);
    expect(chain.describe()).toEqual(["convolution:abc123"]);
    const convolver = created.find((node) => node.kind === "convolver")!;
    expect(convolver.buffer).toBe(impulse);
    expect(convolver.normalize).toBeFalse();
  });

  test("availability says where convolution can run", () => {
    expect(
      dspAvailability({ kind: "client", webAudio: true }).convolution.available,
    ).toBeTrue();
    expect(
      dspAvailability({ kind: "client", webAudio: true, convolver: false })
        .convolution.reason,
    ).toContain("server output");
    expect(dspAvailability({ kind: "cast" }).convolution.available).toBeFalse();
    const server = dspAvailability({
      kind: "server",
      deviceName: "Den Streamer",
      support: {
        available: true,
        reason: null,
        engine: "Parson DSP, 64-bit float",
        crossfeed: true,
        sample_rates: [44_100],
        output_bits: 24,
      },
    });
    expect(server.convolution.available).toBeTrue();
    expect(server.engine).toContain("before Den Streamer");
  });

  test("a context without a convolver skips the stage", () => {
    const { context } = fakeContext(false);
    const settings = defaultDspSettings();
    settings.convolution = { enabled: true, filterId: "abc123" };
    const chain = createDspChain(context, settings, { impulse: () => ({}) });
    expect(chain.describe()).toEqual([]);
  });
});
