import { describe, expect, test } from "bun:test";
import {
  activeEqBands,
  biquadMagnitudeDb,
  createDspChain,
  defaultDspSettings,
  dspAvailability,
  dspMayClip,
  dspNeedsGraph,
  dspProcesses,
  eqPeakGainDb,
  eqPresets,
  normalizeDspSettings,
  recommendedHeadroomDb,
  requestedContextSampleRate,
  type DspAudioContext,
  type DspSettings,
} from "./dsp";
import {
  buildSignalPath,
  overallSignalQuality,
  type ClientSignalLeg,
} from "./signal-path";

function withEq(gains: number[], extra: Partial<DspSettings> = {}) {
  const settings = defaultDspSettings();
  settings.eq.enabled = true;
  settings.eq.bands = settings.eq.bands.map((band, index) => ({
    ...band,
    gainDb: gains[index] ?? 0,
  }));
  return { ...settings, ...extra };
}

/** A recording stand-in for a Web Audio context. */
function fakeContext(sampleRate = 48_000, splitters = true) {
  const edges: [string, string, number, number][] = [];
  let next = 0;
  const node = (kind: string, extra: object = {}) => {
    const name = `${kind}#${next++}`;
    const self = {
      name,
      ...extra,
      connect(to: { name: string }, output = 0, input = 0) {
        edges.push([name, to.name, output, input]);
        return to;
      },
      disconnect() {
        for (let index = edges.length - 1; index >= 0; index -= 1)
          if (edges[index]![0] === name) edges.splice(index, 1);
      },
    };
    return self;
  };
  const param = () => ({ value: 0 });
  const context = {
    sampleRate,
    createGain: () => node("gain", { gain: { value: 1 } }),
    createBiquadFilter: () =>
      node("biquad", {
        type: "lowpass",
        frequency: param(),
        Q: param(),
        gain: param(),
      }),
    createDelay: () => node("delay", { delayTime: param() }),
    ...(splitters
      ? {
          createChannelSplitter: () => node("splitter"),
          createChannelMerger: () => node("merger"),
        }
      : {}),
  };
  return { context: context as unknown as DspAudioContext, edges };
}

/** Follows single-output links from `start` to `end`. */
function chainPath(
  edges: [string, string, number, number][],
  start: string,
  end: string,
) {
  const path = [start];
  let current = start;
  while (current !== end) {
    const out = edges.filter((edge) => edge[0] === current);
    if (out.length !== 1) break;
    current = out[0]![1];
    path.push(current);
  }
  return path.map((name) => name.split("#")[0]);
}

describe("DSP settings", () => {
  test("defaults bypass every section, so nothing is processed", () => {
    const settings = defaultDspSettings();
    expect(settings.enabled).toBeTrue();
    expect(settings.eq.bands).toHaveLength(5);
    expect(dspProcesses(settings)).toBeFalse();
    expect(dspNeedsGraph(settings, 44_100)).toBeFalse();
  });

  test("stored settings are clamped and gaps filled", () => {
    const settings = normalizeDspSettings({
      enabled: true,
      headroom: { enabled: true, gainDb: -90 },
      eq: {
        enabled: true,
        bands: [{ type: "bogus", frequency: 5, gainDb: 40, q: 0 }],
      },
      sampleRate: { enabled: true, mode: 12_345 },
    });
    expect(settings.headroom.gainDb).toBe(-24);
    expect(settings.eq.bands[0]).toEqual({
      id: "band-1",
      enabled: true,
      type: "peaking",
      frequency: 20,
      gainDb: 18,
      q: 0.1,
    });
    expect(settings.sampleRate.mode).toBe("device");
    expect(settings.crossfeed).toEqual({ enabled: false, strength: "medium" });
    expect(normalizeDspSettings("junk")).toEqual(defaultDspSettings());
  });

  test("0 dB peaks are not active bands, pass filters always are", () => {
    const rumble = defaultDspSettings();
    rumble.eq.enabled = true;
    rumble.eq.bands = eqPresets.find((preset) => preset.id === "rumble")!.bands;
    expect(activeEqBands(rumble).map((band) => band.type)).toEqual([
      "highpass",
    ]);
    expect(activeEqBands({ ...rumble, enabled: false })).toEqual([]);
  });

  test("the sample rate choice follows the source or a fixed rate", () => {
    const settings = defaultDspSettings();
    settings.sampleRate = { enabled: true, mode: "source" };
    expect(requestedContextSampleRate(settings, 88_200)).toBe(88_200);
    expect(requestedContextSampleRate(settings, null)).toBeNull();
    settings.sampleRate.mode = 192_000;
    expect(requestedContextSampleRate(settings, 44_100)).toBe(192_000);
    expect(dspNeedsGraph(settings, 44_100)).toBeTrue();
    expect(
      requestedContextSampleRate({ ...settings, enabled: false }, 44_100),
    ).toBeNull();
  });
});

describe("biquad response", () => {
  test("matches the RBJ cookbook at the defining frequencies", () => {
    const peak = {
      type: "peaking",
      frequency: 1_000,
      gainDb: 6,
      q: 1,
    } as const;
    expect(biquadMagnitudeDb(peak, 1_000, 48_000)).toBeCloseTo(6, 3);
    expect(biquadMagnitudeDb(peak, 20, 48_000)).toBeCloseTo(0, 1);
    const shelf = {
      type: "lowshelf",
      frequency: 100,
      gainDb: 5,
      q: 1,
    } as const;
    expect(biquadMagnitudeDb(shelf, 20, 48_000)).toBeCloseTo(5, 0);
    expect(biquadMagnitudeDb(shelf, 100, 48_000)).toBeCloseTo(2.5, 1);
    const highpass = {
      type: "highpass",
      frequency: 1_000,
      gainDb: 0,
      q: Math.SQRT1_2,
    } as const;
    expect(biquadMagnitudeDb(highpass, 1_000, 48_000)).toBeCloseTo(-3.01, 1);
  });

  test("headroom advice covers the largest boost", () => {
    const boosted = withEq([5, 2, 0, 0, 0]);
    // The shelf and the 250 Hz peak overlap a little below 5 dB.
    expect(eqPeakGainDb(boosted)).toBeGreaterThan(4.9);
    expect(recommendedHeadroomDb(boosted)).toBe(-5);
    expect(dspMayClip(boosted)).toBeTrue();
    boosted.headroom = { enabled: true, gainDb: -6 };
    expect(dspMayClip(boosted)).toBeFalse();
    expect(recommendedHeadroomDb(withEq([-3, 0, -2, 0, 0]))).toBe(0);
  });
});

describe("DSP chain graph", () => {
  test("bypassed sections leave input wired straight to output", () => {
    const { context, edges } = fakeContext();
    const chain = createDspChain(context, defaultDspSettings());
    expect(chain.describe()).toEqual([]);
    expect(edges).toEqual([["gain#0", "gain#1", 0, 0]]);
  });

  test("headroom, EQ bands and crossfeed run in order and rebuild on change", () => {
    const { context, edges } = fakeContext();
    const settings = withEq([3, 0, -2, 0, 1.5], {
      headroom: { enabled: true, gainDb: -3 },
      crossfeed: { enabled: true, strength: "strong" },
    });
    const chain = createDspChain(context, settings);
    expect(chain.describe()).toEqual([
      "headroom",
      "eq:lowshelf@80",
      "eq:peaking@1000",
      "eq:highshelf@10000",
      "crossfeed:strong",
    ]);
    const start = (chain.input as unknown as { name: string }).name;
    const end = (chain.output as unknown as { name: string }).name;
    expect(chainPath(edges, start, end)).toEqual([
      "gain",
      "gain",
      "biquad",
      "biquad",
      "biquad",
      "splitter",
    ]);
    // Each ear gets its own direct path and a fed path into the other.
    const merges = edges.filter((edge) => edge[1].startsWith("merger"));
    expect(merges.map((edge) => edge[3]).sort()).toEqual([0, 0, 1, 1]);
    expect(chain.input.channelCount).toBe(2);
    expect(chain.input.channelCountMode).toBe("explicit");

    chain.update({ ...settings, enabled: false });
    expect(chain.describe()).toEqual([]);
    expect(edges.filter((edge) => edge[0] === start)).toEqual([
      [start, end, 0, 0],
    ]);
  });

  test("engines without channel splitters skip crossfeed", () => {
    const { context } = fakeContext(48_000, false);
    const chain = createDspChain(context, {
      ...defaultDspSettings(),
      crossfeed: { enabled: true, strength: "light" },
    });
    expect(chain.describe()).toEqual([]);
  });
});

describe("DSP stages in the signal path", () => {
  // The browser's shared-mode output is high quality on its own, so the
  // chain's own rating is read from the stages before it.
  const chainQuality = (path: ReturnType<typeof buildSignalPath>) =>
    overallSignalQuality(
      path.stages.filter((stage) => stage.kind !== "output"),
    );
  const leg: ClientSignalLeg = {
    platform: "web",
    engine: "web-audio",
    contextSampleRate: 44_100,
    deviceSampleRate: 44_100,
    playbackRate: 1,
    preservePitch: true,
    reverbWet: 0,
    volume: 1,
    output: { kind: "system", name: "Speakers" },
  };
  const audio = {
    codec: "flac",
    container: "flac",
    sample_rate: 44_100,
    bit_depth: 24,
    channels: 2,
    bitrate_kbps: 1_200,
    lossless: true,
  };

  test("each node is a stage with its parameters, rated enhanced", () => {
    const dsp = withEq([5, 2, 0, -1, 3], {
      headroom: { enabled: true, gainDb: -3 },
      crossfeed: { enabled: true, strength: "medium" },
    });
    const path = buildSignalPath(null, { ...leg, dsp }, audio);
    expect(path.stages.map((stage) => stage.id)).toEqual([
      "source",
      "client-decode",
      "client-depth-in",
      "dsp-headroom",
      "dsp-eq",
      "dsp-crossfeed",
      "client-depth-out",
      "client-output",
    ]);
    const stage = (id: string) => path.stages.find((item) => item.id === id)!;
    expect(stage("client-depth-in").detail).toBe("24bit to 32bit float");
    expect(stage("dsp-headroom").detail).toBe("−3.0 dB");
    expect(stage("dsp-eq").detail).toStartWith(
      "4 bands: 80 Hz low shelf +5.0 dB, 250 Hz peak +2.0 dB",
    );
    expect(stage("dsp-eq").explanation).toContain("turn on headroom");
    expect(stage("dsp-crossfeed").detail).toBe(
      "Medium: 700 Hz, −6.0 dB, 0.3 ms",
    );
    expect(chainQuality(path)).toBe("enhanced");
    expect(path.quality).toBe("high_quality");
    expect(path.description).toContain("Parametric EQ (4 bands");
  });

  test("the master switch removes every DSP stage", () => {
    const dsp = withEq([5, 0, 0, 0, 0], { enabled: false });
    const path = buildSignalPath(null, { ...leg, dsp }, audio);
    expect(path.stages.some((stage) => stage.id.startsWith("dsp-"))).toBe(
      false,
    );
    expect(chainQuality(path)).toBe("lossless");
    expect(path.bitPerfectToOutput).toBeTrue();
  });

  test("a chosen upsampling rate is enhanced, a forced one is not", () => {
    const dsp = defaultDspSettings();
    dsp.sampleRate = { enabled: true, mode: 96_000 };
    const chosen = buildSignalPath(
      null,
      { ...leg, dsp, contextSampleRate: 96_000, deviceSampleRate: 96_000 },
      audio,
    );
    expect(
      chosen.stages.find((stage) => stage.id === "client-resample"),
    ).toEqual(
      expect.objectContaining({
        detail: "44.1kHz to 96kHz",
        quality: "enhanced",
      }),
    );
    expect(chainQuality(chosen)).toBe("enhanced");
    const forced = buildSignalPath(
      null,
      { ...leg, contextSampleRate: 48_000, deviceSampleRate: 48_000 },
      audio,
    );
    expect(chainQuality(forced)).toBe("high_quality");
  });
});

describe("where the DSP chain can run", () => {
  test("a server output runs it in 64-bit float within the device's rates", () => {
    const availability = dspAvailability({
      kind: "server",
      deviceName: "USB DAC",
      support: {
        available: true,
        reason: null,
        engine: "Mujin Music DSP, 64-bit float",
        crossfeed: true,
        sample_rates: [44_100, 48_000, 96_000],
        output_bits: 32,
      },
    });
    expect(availability.available).toBeTrue();
    expect(availability.precision).toBe("64-bit float");
    expect(availability.engine).toBe(
      "Mujin Music DSP, 64-bit float, on the server before USB DAC",
    );
    expect(availability.sampleRates).toEqual([44_100, 48_000, 96_000]);
    expect(availability.crossfeed.available).toBeTrue();
  });

  test("controls are greyed only where the chain truly cannot apply", () => {
    const unprobed = dspAvailability({
      kind: "server",
      deviceName: "USB DAC",
      support: {
        available: false,
        reason: "Mujin Music could not read the formats USB DAC accepts.",
        engine: "Mujin Music DSP, 64-bit float",
        crossfeed: false,
        sample_rates: [],
        output_bits: null,
      },
    });
    expect(unprobed.available).toBeFalse();
    expect(unprobed.reason).toContain("could not read");
    const mono = dspAvailability({
      kind: "server",
      deviceName: "Mono speaker",
      support: {
        available: true,
        reason: null,
        engine: "Mujin Music DSP, 64-bit float",
        crossfeed: false,
        sample_rates: [48_000],
        output_bits: 16,
      },
    });
    expect(mono.available).toBeTrue();
    expect(mono.crossfeed).toEqual({
      available: false,
      reason: "Mono speaker does not open in stereo, which crossfeed needs.",
    });
    expect(
      dspAvailability({ kind: "server", deviceName: "X", support: undefined })
        .available,
    ).toBeFalse();
    expect(dspAvailability({ kind: "cast" }).available).toBeFalse();
    expect(
      dspAvailability({ kind: "client", webAudio: false }).available,
    ).toBeFalse();
    const browser = dspAvailability({ kind: "client", webAudio: true });
    expect(browser.available).toBeTrue();
    expect(browser.precision).toBe("32-bit float");
    expect(browser.sampleRates).toBeNull();
  });
});
