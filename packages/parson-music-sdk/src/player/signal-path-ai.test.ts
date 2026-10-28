import { expect, test } from "bun:test";
import { normalizeDspSettings, defaultDspSettings } from "./dsp";
import {
  AI_ENHANCE_STAGE,
  aiEnhancerModel,
  carryLossyGeneration,
  describeAudioFormat,
  describeCollectionFormat,
  describeSignalPath,
  isAiEnhancedPath,
  overallSignalQuality,
  sourceQuality,
  type SignalStage,
} from "./signal-path";

const stage = (overrides: Partial<SignalStage>): SignalStage => ({
  id: "x",
  kind: "dsp",
  title: "Stage",
  detail: "",
  quality: "lossless",
  explanation: "",
  ...overrides,
});

test("real-time AI enhancement starts a new generation rated enhanced", () => {
  const stages = carryLossyGeneration([
    stage({
      id: "source",
      kind: "source",
      detail: "MP3 320kbps",
      quality: "low_quality",
    }),
    stage({ id: "native-decode", kind: "decode", quality: "lossless" }),
    stage({
      id: AI_ENHANCE_STAGE,
      title: "AI-enhanced (Apollo)",
      quality: "enhanced",
    }),
    stage({ id: "native-output", kind: "output", quality: "lossless" }),
  ]);
  expect(stages.map((item) => item.quality)).toEqual([
    "low_quality",
    "low_quality",
    "enhanced",
    "lossless",
  ]);
  expect(overallSignalQuality(stages)).toBe("enhanced");
  expect(isAiEnhancedPath(stages)).toBeTrue();
  const sentence = describeSignalPath(stages);
  expect(sentence).toContain("AI-enhanced audio: Apollo");
  expect(sentence).toContain("not lossless");
});

test("an AI-enhanced file is described as such, never as lossless", () => {
  const stages = [
    stage({
      id: "source",
      kind: "source",
      title: "Source",
      detail: "AI-enhanced (Apollo) · FLAC 44.1kHz 24bit 2ch",
      quality: "enhanced",
    }),
    stage({ id: "transport", kind: "transport", quality: "lossless" }),
  ];
  expect(overallSignalQuality(stages)).toBe("enhanced");
  expect(describeSignalPath(stages)).toContain("Apollo");
  // Without AI, an MP3 path stays low quality throughout.
  const plain = carryLossyGeneration([
    stage({ id: "source", kind: "source", quality: "low_quality" }),
    stage({ id: "out", kind: "output", quality: "lossless" }),
  ]);
  expect(overallSignalQuality(plain)).toBe("low_quality");
});

test("AI enhance is off unless stored as on", () => {
  expect(defaultDspSettings().aiEnhance).toEqual({ enabled: false });
  expect(normalizeDspSettings({}).aiEnhance).toEqual({ enabled: false });
  expect(
    normalizeDspSettings({ aiEnhance: { enabled: true } }).aiEnhance,
  ).toEqual({ enabled: true });
});

test("AI-enhanced files are never shown as lossless or hi-res", () => {
  const enhanced = {
    codec: "flac",
    container: "flac",
    sample_rate: 44_100,
    bit_depth: 24,
    channels: 2,
    bitrate_kbps: 900,
    lossless: true,
    enhanced_by: "Apollo",
  };
  const real = { ...enhanced, enhanced_by: null };
  expect(sourceQuality(enhanced)).toBe("enhanced");
  expect(sourceQuality(real)).toBe("lossless");
  const format = describeAudioFormat(enhanced)!;
  expect(format.high_res).toBeFalse();
  expect(format.lossless).toBeFalse();
  expect(format.summary).toBe("AI-enhanced (Apollo) · FLAC 44.1kHz 24bit 2ch");
  expect(describeAudioFormat(real)!.high_res).toBeTrue();
  const album = describeCollectionFormat([enhanced, enhanced])!;
  expect(album).toEqual({
    summary: "AI-enhanced · FLAC 44.1kHz 24bit",
    quality: "enhanced",
    highRes: false,
  });
  const mixed = describeCollectionFormat([enhanced, real])!;
  expect(mixed.summary.startsWith("Partly AI-enhanced · ")).toBeTrue();
  expect(mixed.quality).toBe("enhanced");
  expect(mixed.highRes).toBeTrue();
});

test("reads the enhancer model from a stage label in linear time", () => {
  expect(aiEnhancerModel("Source AI-enhanced (Apollo) 24-bit")).toBe("Apollo");
  expect(aiEnhancerModel("AI-enhanced ()")).toBeUndefined();
  expect(aiEnhancerModel("lossless FLAC")).toBeUndefined();
  const hostile = "AI-enhanced (" + "AI-enhanced ((".repeat(50_000);
  const started = performance.now();
  expect(aiEnhancerModel(hostile)).toBeUndefined();
  expect(performance.now() - started).toBeLessThan(100);
});
