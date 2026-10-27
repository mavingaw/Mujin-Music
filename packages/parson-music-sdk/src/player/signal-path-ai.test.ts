import { expect, test } from "bun:test";
import { normalizeDspSettings, defaultDspSettings } from "./dsp";
import {
  AI_ENHANCE_STAGE,
  carryLossyGeneration,
  describeSignalPath,
  isAiEnhancedPath,
  overallSignalQuality,
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
