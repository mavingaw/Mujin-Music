/**
 * A per-output DSP chain in the spirit of Roon's MUSE: headroom
 * management, a parametric EQ made of biquad filters, headphone crossfeed,
 * convolution with an uploaded FIR filter (room correction) and a sample
 * rate choice. Each section has its own switch and the whole
 * chain has a master switch. The settings are plain data so web and
 * mobile share them; `createDspChain` builds the nodes on any Web Audio
 * style context (the browser's or react-native-audio-api's).
 */

export type EqFilterType =
  "peaking" | "lowshelf" | "highshelf" | "lowpass" | "highpass" | "notch";

export interface EqBand {
  id: string;
  enabled: boolean;
  type: EqFilterType;
  /** Centre or corner frequency in Hz. */
  frequency: number;
  /** Ignored by pass and notch filters. */
  gainDb: number;
  q: number;
}

export type CrossfeedStrength = "light" | "medium" | "strong";

/** `device` keeps the output's own rate; `source` follows each track. */
export type DspSampleRateMode = "device" | "source" | number;

export interface DspSettings {
  /** Master switch: off bypasses every section. */
  enabled: boolean;
  headroom: { enabled: boolean; gainDb: number };
  eq: { enabled: boolean; preset: string; bands: EqBand[] };
  crossfeed: { enabled: boolean; strength: CrossfeedStrength };
  /**
   * An FIR filter (room correction and the like) from the filters stored
   * on the server (older settings omit it: off).
   */
  convolution: { enabled: boolean; filterId: string | null };
  sampleRate: { enabled: boolean; mode: DspSampleRateMode };
  /**
   * AI enhance in real time (experimental): lossy tracks on a server output
   * stream through your enhancer worker. It generates plausible
   * treble; it never recovers the original master. Older settings omit it.
   */
  aiEnhance?: { enabled: boolean };
}

export const DSP_SAMPLE_RATES = [
  44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
] as const;

export const EQ_FILTER_TYPES: readonly EqFilterType[] = [
  "peaking",
  "lowshelf",
  "highshelf",
  "lowpass",
  "highpass",
  "notch",
];

export const eqFilterTypeLabels: Record<EqFilterType, string> = {
  peaking: "Peak",
  lowshelf: "Low shelf",
  highshelf: "High shelf",
  lowpass: "Low pass",
  highpass: "High pass",
  notch: "Notch",
};

export const crossfeedStrengthLabels: Record<CrossfeedStrength, string> = {
  light: "Light",
  medium: "Medium",
  strong: "Strong",
};

const MIN_FREQUENCY = 20;
const MAX_FREQUENCY = 20_000;
const MIN_HEADROOM_DB = -24;
const MAX_EQ_GAIN_DB = 18;

type BandShape = Omit<EqBand, "id" | "enabled">;

function bands(shapes: BandShape[]): EqBand[] {
  return shapes.map((shape, index) => ({
    id: `band-${index + 1}`,
    enabled: true,
    ...shape,
  }));
}

/** Five bands, like Roon's default parametric EQ. */
const FIVE_BAND_FREQUENCIES = [80, 250, 1_000, 4_000, 10_000];

function fiveBand(gains: [number, number, number, number, number]) {
  return bands(
    FIVE_BAND_FREQUENCIES.map((frequency, index) => ({
      type: index === 0 ? "lowshelf" : index === 4 ? "highshelf" : "peaking",
      frequency,
      gainDb: gains[index]!,
      q: index === 0 || index === 4 ? 0.71 : 1.0,
    })),
  );
}

export interface EqPreset {
  id: string;
  label: string;
  bands: EqBand[];
}

export const eqPresets: readonly EqPreset[] = [
  { id: "flat", label: "Flat", bands: fiveBand([0, 0, 0, 0, 0]) },
  { id: "bass", label: "Bass boost", bands: fiveBand([5, 2, 0, 0, 0]) },
  { id: "treble", label: "Treble boost", bands: fiveBand([0, 0, 0, 2, 4]) },
  { id: "vocal", label: "Vocal", bands: fiveBand([-2, -1, 2.5, 2, 0]) },
  { id: "loudness", label: "Loudness", bands: fiveBand([4, 1, -1, 1, 3]) },
  {
    id: "rumble",
    label: "Rumble filter",
    bands: bands([
      { type: "highpass", frequency: 30, gainDb: 0, q: 0.71 },
      ...fiveBand([0, 0, 0, 0, 0]).slice(1),
    ]),
  },
];

export function defaultDspSettings(): DspSettings {
  return {
    enabled: true,
    headroom: { enabled: false, gainDb: -3 },
    eq: { enabled: false, preset: "flat", bands: fiveBand([0, 0, 0, 0, 0]) },
    crossfeed: { enabled: false, strength: "medium" },
    convolution: { enabled: false, filterId: null },
    sampleRate: { enabled: false, mode: "device" },
    aiEnhance: { enabled: false },
  };
}

/** Filter ids as the server writes them: hex and dashes. */
export function validFilterId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 64 &&
    /^[0-9a-fA-F-]+$/.test(value)
  );
}

function finite(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

function normalizeBand(value: unknown, index: number): EqBand {
  const band = record(value);
  const type = EQ_FILTER_TYPES.includes(band.type as EqFilterType)
    ? (band.type as EqFilterType)
    : "peaking";
  return {
    id: typeof band.id === "string" && band.id ? band.id : `band-${index + 1}`,
    enabled: band.enabled !== false,
    type,
    frequency: clamp(
      finite(band.frequency, 1_000),
      MIN_FREQUENCY,
      MAX_FREQUENCY,
    ),
    gainDb: clamp(finite(band.gainDb, 0), -MAX_EQ_GAIN_DB, MAX_EQ_GAIN_DB),
    q: clamp(finite(band.q, 1), 0.1, 18),
  };
}

/** Parses stored settings, filling gaps and clamping every value. */
export function normalizeDspSettings(value: unknown): DspSettings {
  const defaults = defaultDspSettings();
  const input = record(value);
  const headroom = record(input.headroom);
  const eq = record(input.eq);
  const crossfeed = record(input.crossfeed);
  const convolution = record(input.convolution);
  const sampleRate = record(input.sampleRate);
  const aiEnhance = record(input.aiEnhance);
  const rawBands = Array.isArray(eq.bands) ? eq.bands.slice(0, 10) : null;
  const mode = sampleRate.mode;
  return {
    enabled: input.enabled !== false,
    headroom: {
      enabled: headroom.enabled === true,
      gainDb: clamp(
        finite(headroom.gainDb, defaults.headroom.gainDb),
        MIN_HEADROOM_DB,
        0,
      ),
    },
    eq: {
      enabled: eq.enabled === true,
      preset:
        typeof eq.preset === "string"
          ? eq.preset
          : rawBands
            ? "custom"
            : defaults.eq.preset,
      bands: rawBands
        ? rawBands.map((band, index) => normalizeBand(band, index))
        : defaults.eq.bands,
    },
    crossfeed: {
      enabled: crossfeed.enabled === true,
      strength:
        crossfeed.strength === "light" || crossfeed.strength === "strong"
          ? crossfeed.strength
          : "medium",
    },
    convolution: {
      enabled: convolution.enabled === true,
      filterId: validFilterId(convolution.filterId)
        ? convolution.filterId
        : null,
    },
    sampleRate: {
      enabled: sampleRate.enabled === true,
      mode:
        mode === "source" ||
        (typeof mode === "number" &&
          (DSP_SAMPLE_RATES as readonly number[]).includes(mode))
          ? mode
          : "device",
    },
    aiEnhance: { enabled: aiEnhance.enabled === true },
  };
}

/** Bands that change the sound: enabled, and not a 0 dB peak or shelf. */
export function activeEqBands(settings: DspSettings): EqBand[] {
  if (!settings.enabled || !settings.eq.enabled) return [];
  return settings.eq.bands.filter(
    (band) =>
      band.enabled &&
      (!["peaking", "lowshelf", "highshelf"].includes(band.type) ||
        Math.abs(band.gainDb) >= 0.05),
  );
}

export function headroomActive(settings: DspSettings) {
  return (
    settings.enabled &&
    settings.headroom.enabled &&
    settings.headroom.gainDb <= -0.05
  );
}

export function crossfeedActive(settings: DspSettings) {
  return settings.enabled && settings.crossfeed.enabled;
}

/** The convolution filter the chain runs, or `null`. */
export function activeConvolutionFilter(settings: DspSettings): string | null {
  return settings.enabled &&
    settings.convolution?.enabled &&
    settings.convolution.filterId
    ? settings.convolution.filterId
    : null;
}

/**
 * Sample rate the audio context should run at, or `null` for the output
 * device's own rate.
 */
export function requestedContextSampleRate(
  settings: DspSettings | null | undefined,
  sourceRate: number | null | undefined,
): number | null {
  if (!settings?.enabled || !settings.sampleRate.enabled) return null;
  const mode = settings.sampleRate.mode;
  if (mode === "device") return null;
  if (mode === "source")
    return sourceRate && sourceRate > 0 ? sourceRate : null;
  return mode;
}

/** Any section that changes the samples. */
export function dspProcesses(settings: DspSettings | null | undefined) {
  if (!settings) return false;
  return (
    headroomActive(settings) ||
    activeEqBands(settings).length > 0 ||
    crossfeedActive(settings) ||
    activeConvolutionFilter(settings) !== null
  );
}

/** The chain needs a Web Audio graph: it processes or picks a rate. */
export function dspNeedsGraph(
  settings: DspSettings | null | undefined,
  sourceRate?: number | null,
) {
  return (
    dspProcesses(settings) ||
    requestedContextSampleRate(settings, sourceRate) !== null
  );
}

export interface CrossfeedParameters {
  /** Low-pass corner of the signal fed to the other ear. */
  cutoffHz: number;
  /** Level of the fed signal relative to the direct one. */
  feedDb: number;
  /** Interaural delay. */
  delayMs: number;
}

/** Bauer-style crossfeed levels (light ≈ Chu Moy, strong ≈ Jan Meier). */
export const crossfeedParameters: Record<
  CrossfeedStrength,
  CrossfeedParameters
> = {
  light: { cutoffHz: 700, feedDb: -9.5, delayMs: 0.3 },
  medium: { cutoffHz: 700, feedDb: -6, delayMs: 0.3 },
  strong: { cutoffHz: 650, feedDb: -4.5, delayMs: 0.35 },
};

/**
 * Magnitude response of one biquad in dB, using the RBJ Audio EQ Cookbook
 * formulas that Web Audio's BiquadFilterNode implements.
 */
export function biquadMagnitudeDb(
  band: Pick<EqBand, "type" | "frequency" | "gainDb" | "q">,
  frequency: number,
  sampleRate: number,
): number {
  const nyquist = sampleRate / 2;
  const f0 = Math.min(band.frequency, nyquist * 0.999);
  const w0 = (2 * Math.PI * f0) / sampleRate;
  const cos = Math.cos(w0);
  const sin = Math.sin(w0);
  const A = 10 ** (band.gainDb / 40);
  const Q = Math.max(band.q, 1e-4);
  let b0: number, b1: number, b2: number, a0: number, a1: number, a2: number;
  switch (band.type) {
    case "lowpass": {
      const alpha = sin / (2 * Q);
      b0 = (1 - cos) / 2;
      b1 = 1 - cos;
      b2 = (1 - cos) / 2;
      a0 = 1 + alpha;
      a1 = -2 * cos;
      a2 = 1 - alpha;
      break;
    }
    case "highpass": {
      const alpha = sin / (2 * Q);
      b0 = (1 + cos) / 2;
      b1 = -(1 + cos);
      b2 = (1 + cos) / 2;
      a0 = 1 + alpha;
      a1 = -2 * cos;
      a2 = 1 - alpha;
      break;
    }
    case "notch": {
      const alpha = sin / (2 * Q);
      b0 = 1;
      b1 = -2 * cos;
      b2 = 1;
      a0 = 1 + alpha;
      a1 = -2 * cos;
      a2 = 1 - alpha;
      break;
    }
    case "lowshelf":
    case "highshelf": {
      // Web Audio ignores Q for shelves and uses a slope of 1.
      const alpha = (sin / 2) * Math.SQRT2;
      const root = 2 * Math.sqrt(A) * alpha;
      if (band.type === "lowshelf") {
        b0 = A * (A + 1 - (A - 1) * cos + root);
        b1 = 2 * A * (A - 1 - (A + 1) * cos);
        b2 = A * (A + 1 - (A - 1) * cos - root);
        a0 = A + 1 + (A - 1) * cos + root;
        a1 = -2 * (A - 1 + (A + 1) * cos);
        a2 = A + 1 + (A - 1) * cos - root;
      } else {
        b0 = A * (A + 1 + (A - 1) * cos + root);
        b1 = -2 * A * (A - 1 + (A + 1) * cos);
        b2 = A * (A + 1 + (A - 1) * cos - root);
        a0 = A + 1 - (A - 1) * cos + root;
        a1 = 2 * (A - 1 - (A + 1) * cos);
        a2 = A + 1 - (A - 1) * cos - root;
      }
      break;
    }
    default: {
      const alpha = sin / (2 * Q);
      b0 = 1 + alpha * A;
      b1 = -2 * cos;
      b2 = 1 - alpha * A;
      a0 = 1 + alpha / A;
      a1 = -2 * cos;
      a2 = 1 - alpha / A;
    }
  }
  const w = (2 * Math.PI * frequency) / sampleRate;
  const cos1 = Math.cos(w);
  const sin1 = Math.sin(w);
  const cos2 = Math.cos(2 * w);
  const sin2 = Math.sin(2 * w);
  const numRe = b0 + b1 * cos1 + b2 * cos2;
  const numIm = -(b1 * sin1 + b2 * sin2);
  const denRe = a0 + a1 * cos1 + a2 * cos2;
  const denIm = -(a1 * sin1 + a2 * sin2);
  const magnitude = Math.sqrt(
    (numRe * numRe + numIm * numIm) / (denRe * denRe + denIm * denIm),
  );
  return 20 * Math.log10(Math.max(magnitude, 1e-12));
}

/** Combined EQ response at `frequency`. */
export function eqResponseDb(
  bandsToSum: readonly EqBand[],
  frequency: number,
  sampleRate = 48_000,
) {
  return bandsToSum.reduce(
    (sum, band) => sum + biquadMagnitudeDb(band, frequency, sampleRate),
    0,
  );
}

/** Log-spaced frequencies from 20 Hz to 20 kHz, for curves and peaks. */
export function eqFrequencies(points = 96) {
  const low = Math.log10(MIN_FREQUENCY);
  const high = Math.log10(MAX_FREQUENCY);
  return Array.from(
    { length: points },
    (_, index) => 10 ** (low + ((high - low) * index) / (points - 1)),
  );
}

/** Largest boost the active EQ applies anywhere in the audible band. */
export function eqPeakGainDb(settings: DspSettings, sampleRate = 48_000) {
  const active = activeEqBands(settings);
  if (!active.length) return 0;
  return Math.max(
    0,
    ...eqFrequencies(240).map((frequency) =>
      eqResponseDb(active, frequency, sampleRate),
    ),
  );
}

/**
 * Headroom that keeps the EQ's largest boost from clipping a full-scale
 * track, rounded down to half a decibel; `0` when the EQ only cuts.
 */
export function recommendedHeadroomDb(settings: DspSettings) {
  const peak = eqPeakGainDb(settings);
  return peak <= 0.05 ? 0 : -Math.ceil(peak * 2) / 2;
}

/**
 * Headroom is too small for the EQ boost: loud passages can clip after
 * the chain.
 */
export function dspMayClip(settings: DspSettings) {
  const needed = recommendedHeadroomDb(settings);
  if (needed === 0) return false;
  const applied = headroomActive(settings) ? settings.headroom.gainDb : 0;
  return applied > needed + 0.01;
}

export function formatFrequency(frequency: number) {
  if (frequency >= 1_000) {
    const khz = frequency / 1_000;
    return `${Number.isInteger(khz) ? khz : khz.toFixed(1)} kHz`;
  }
  return `${Math.round(frequency)} Hz`;
}

function formatGain(db: number) {
  const rounded = Math.round(db * 10) / 10;
  return `${rounded > 0 ? "+" : rounded < 0 ? "−" : ""}${Math.abs(rounded).toFixed(1)} dB`;
}

/** e.g. `80 Hz low shelf +5.0 dB`. */
export function describeEqBand(band: EqBand) {
  const where = formatFrequency(band.frequency);
  switch (band.type) {
    case "lowpass":
    case "highpass":
    case "notch":
      return `${where} ${eqFilterTypeLabels[band.type].toLowerCase()}`;
    default:
      return `${where} ${band.type === "peaking" ? "peak" : eqFilterTypeLabels[band.type].toLowerCase()} ${formatGain(band.gainDb)}`;
  }
}

/* ------------------------------------------------------------------ */
/* Web Audio graph                                                     */
/* ------------------------------------------------------------------ */

export interface DspAudioParam {
  value: number;
}

export interface DspAudioNode {
  connect(destination: never, output?: number, input?: number): unknown;
  disconnect(): void;
}

export interface DspGainNode extends DspAudioNode {
  gain: DspAudioParam;
  channelCount?: number;
  channelCountMode?: string;
  channelInterpretation?: string;
}

export interface DspBiquadNode extends DspAudioNode {
  type: string;
  frequency: DspAudioParam;
  Q: DspAudioParam;
  gain: DspAudioParam;
}

/** A Web Audio `ConvolverNode`. */
export interface DspConvolverNode extends DspAudioNode {
  buffer: unknown;
  normalize: boolean;
}

export interface DspDelayNode extends DspAudioNode {
  delayTime: DspAudioParam;
}

/** The parts of a Web Audio context the chain uses. */
export interface DspAudioContext {
  readonly sampleRate: number;
  createGain(): DspGainNode;
  createBiquadFilter(): DspBiquadNode;
  createDelay(maxDelayTime?: number): DspDelayNode;
  createChannelSplitter?(outputs?: number): DspAudioNode;
  createChannelMerger?(inputs?: number): DspAudioNode;
  createConvolver?(): DspConvolverNode;
}

export interface DspChainOptions {
  /**
   * The decoded impulse response for a filter id (an `AudioBuffer` at the
   * context's rate), or `null` while it is not loaded.
   */
  impulse?: (filterId: string) => unknown;
}

export interface DspChain {
  /** Connect the source here. */
  input: DspGainNode;
  /** Connect this to the rest of the graph. */
  output: DspGainNode;
  /** Rebuilds the processing nodes between input and output. */
  update(settings: DspSettings): void;
  /** Names of the nodes currently between input and output, in order. */
  describe(): string[];
  dispose(): void;
}

type Link = (
  from: DspAudioNode,
  to: DspAudioNode,
  output?: number,
  input?: number,
) => void;

const link: Link = (from, to, output, input) => {
  (from.connect as (to: unknown, output?: number, input?: number) => unknown)(
    to,
    output ?? 0,
    input ?? 0,
  );
};

/**
 * Builds the chain input → headroom → EQ bands → crossfeed →
 * convolution → output.
 * Bypassed sections are left out of the graph entirely, so the master
 * switch off leaves input wired straight to output.
 */
export function createDspChain(
  context: DspAudioContext,
  initial: DspSettings,
  options: DspChainOptions = {},
): DspChain {
  const input = context.createGain();
  const output = context.createGain();
  // Upmix mono to stereo so crossfeed always has two channels. Native
  // engines may expose these as read-only, which is fine to skip.
  try {
    input.channelCount = 2;
    input.channelCountMode = "explicit";
    input.channelInterpretation = "speakers";
  } catch {}
  let nodes: DspAudioNode[] = [];
  let names: string[] = [];

  const build = (settings: DspSettings) => {
    input.disconnect();
    for (const node of nodes) node.disconnect();
    nodes = [];
    names = [];
    let tail: DspAudioNode = input;

    if (headroomActive(settings)) {
      const gain = context.createGain();
      gain.gain.value = 10 ** (settings.headroom.gainDb / 20);
      link(tail, gain);
      nodes.push(gain);
      names.push("headroom");
      tail = gain;
    }

    for (const band of activeEqBands(settings)) {
      const filter = context.createBiquadFilter();
      filter.type = band.type;
      filter.frequency.value = Math.min(
        band.frequency,
        context.sampleRate / 2 - 1,
      );
      filter.Q.value = band.q;
      filter.gain.value = band.gainDb;
      link(tail, filter);
      nodes.push(filter);
      names.push(`eq:${band.type}@${band.frequency}`);
      tail = filter;
    }

    if (
      crossfeedActive(settings) &&
      context.createChannelSplitter &&
      context.createChannelMerger
    ) {
      const params = crossfeedParameters[settings.crossfeed.strength];
      const feed = 10 ** (params.feedDb / 20);
      const splitter = context.createChannelSplitter(2);
      const merger = context.createChannelMerger(2);
      link(tail, splitter);
      nodes.push(splitter, merger);
      for (const channel of [0, 1]) {
        const direct = context.createGain();
        // Keep the summed level of a centred source unchanged.
        direct.gain.value = 1 / (1 + feed);
        link(splitter, direct, channel);
        link(direct, merger, 0, channel);
        const lowpass = context.createBiquadFilter();
        lowpass.type = "lowpass";
        lowpass.frequency.value = params.cutoffHz;
        lowpass.Q.value = Math.SQRT1_2;
        lowpass.gain.value = 0;
        const delay = context.createDelay(0.01);
        delay.delayTime.value = params.delayMs / 1000;
        const cross = context.createGain();
        cross.gain.value = feed / (1 + feed);
        link(splitter, lowpass, channel);
        link(lowpass, delay);
        link(delay, cross);
        link(cross, merger, 0, 1 - channel);
        nodes.push(direct, lowpass, delay, cross);
      }
      names.push(`crossfeed:${settings.crossfeed.strength}`);
      tail = merger;
    }

    const filterId = activeConvolutionFilter(settings);
    const impulse = filterId ? options.impulse?.(filterId) : null;
    if (filterId && impulse && context.createConvolver) {
      const convolver = context.createConvolver();
      // The filter's own gain is the point (room correction); never
      // rescale it.
      convolver.normalize = false;
      convolver.buffer = impulse;
      link(tail, convolver);
      nodes.push(convolver);
      names.push(`convolution:${filterId}`);
      tail = convolver;
    }

    link(tail, output);
  };

  build(initial);
  return {
    input,
    output,
    update: build,
    describe: () => [...names],
    dispose: () => {
      input.disconnect();
      for (const node of nodes) node.disconnect();
      output.disconnect();
      nodes = [];
      names = [];
    },
  };
}

/* ------------------------------------------------------------------ */
/* Where the chain runs, and what it can do there                      */
/* ------------------------------------------------------------------ */

/** What the server reports a server output's chain can do. */
export interface ServerDspCapabilities {
  available: boolean;
  reason: string | null;
  engine: string;
  crossfeed: boolean;
  sample_rates: number[];
  output_bits: number | null;
}

export type DspTarget =
  /** This browser or phone's Web Audio graph (shared-mode output). */
  | {
      kind: "client";
      webAudio: boolean;
      channelSplitting?: boolean;
      /** The engine has a `ConvolverNode` (`false` hides convolution). */
      convolver?: boolean;
    }
  /** A server output: the server runs the chain before the device. */
  | {
      kind: "server";
      deviceName: string;
      support: ServerDspCapabilities | null | undefined;
    }
  /** A Cast receiver decodes the stream itself. */
  | { kind: "cast" };

export interface DspAvailability {
  /** The chain can run at all; when `false`, every control is greyed. */
  available: boolean;
  reason: string | null;
  /** Where and in what precision it runs, for the panel's header. */
  engine: string;
  /** e.g. `64-bit float` or `32-bit float`. */
  precision: string;
  crossfeed: { available: boolean; reason: string | null };
  convolution: { available: boolean; reason: string | null };
  /** Fixed rates that can be chosen; `null` when any listed rate can. */
  sampleRates: readonly number[] | null;
  /** `device` means the server output's own (track's) rate. */
  deviceRateLabel: string;
}

/**
 * Whether, where and how the DSP chain applies for an output, so the
 * panel can grey out what truly cannot run instead of pretending.
 */
export function dspAvailability(target: DspTarget): DspAvailability {
  if (target.kind === "cast") {
    return {
      available: false,
      reason:
        "A Cast receiver fetches and decodes the stream itself, so Mujin Music cannot process it.",
      engine: "Not available while casting",
      precision: "",
      crossfeed: { available: false, reason: null },
      convolution: { available: false, reason: null },
      sampleRates: [],
      deviceRateLabel: "Output device rate",
    };
  }
  if (target.kind === "server") {
    const support = target.support;
    if (!support) {
      return {
        available: false,
        reason: `This server does not run a DSP engine for ${target.deviceName}. Update Mujin Music on the server.`,
        engine: "Server output",
        precision: "",
        crossfeed: { available: false, reason: null },
        convolution: { available: false, reason: null },
        sampleRates: [],
        deviceRateLabel: "Device rate",
      };
    }
    return {
      available: support.available,
      reason: support.available ? null : support.reason,
      engine: `${support.engine}, on the server before ${target.deviceName}`,
      precision: "64-bit float",
      crossfeed: support.crossfeed
        ? { available: true, reason: null }
        : {
            available: false,
            reason: `${target.deviceName} does not open in stereo, which crossfeed needs.`,
          },
      convolution: { available: support.available, reason: null },
      sampleRates: DSP_SAMPLE_RATES.filter((rate) =>
        support.sample_rates.includes(rate),
      ),
      deviceRateLabel: "Track's rate (as the device allows)",
    };
  }
  if (!target.webAudio) {
    return {
      available: false,
      reason:
        "This browser has no Web Audio engine, so the DSP chain cannot run here. Play on a server output to process on the server.",
      engine: "Not available in this browser",
      precision: "",
      crossfeed: { available: false, reason: null },
      convolution: { available: false, reason: null },
      sampleRates: [],
      deviceRateLabel: "Output device rate",
    };
  }
  return {
    available: true,
    reason: null,
    engine: "Web Audio graph in this browser, 32-bit float",
    precision: "32-bit float",
    crossfeed:
      target.channelSplitting === false
        ? {
            available: false,
            reason: "This audio engine cannot split channels.",
          }
        : { available: true, reason: null },
    convolution:
      target.convolver === false
        ? {
            available: false,
            reason:
              "This audio engine has no convolver. Play on a server output to convolve on the server.",
          }
        : { available: true, reason: null },
    sampleRates: null,
    deviceRateLabel: "Output device rate",
  };
}
