import { expect, test } from "bun:test";
import {
  signalPathCaveat,
  signalPathStages,
  stepIcon,
} from "./zone-signal-path-model";

test("a Sonos signal path runs from the file through each step to the room", () => {
  const stages = signalPathStages({
    source: "FLAC 96 kHz/24-bit",
    output: "FLAC 48 kHz/24-bit to Living Room (Sonos Five)",
    bit_perfect: false,
    steps: [
      "Volume leveling -6.4 dB",
      "Sample rate conversion 96 kHz to 48 kHz (linear phase)",
      "Re-encoded as FLAC (lossless)",
    ],
    quality: "high",
  });
  expect(stages.map((stage) => [stage.kicker, stage.icon])).toEqual([
    ["Source", "source"],
    [null, "gain"],
    [null, "rate"],
    [null, "encode"],
    ["Output", "output"],
  ]);
  expect(stages.at(-1)?.label).toBe(
    "FLAC 48 kHz/24-bit to Living Room (Sonos Five)",
  );
});

test("step icons follow the server's wording", () => {
  expect(stepIcon("Decoded Ogg Vorbis to 24-bit PCM")).toBe("decode");
  expect(stepIcon("Bit depth 24-bit to 16-bit (TPDF dither)")).toBe("depth");
  expect(stepIcon("Reduced to 24-bit (the device takes at most 24-bit)")).toBe(
    "depth",
  );
  expect(stepIcon("Resampled 192 kHz to 96 kHz")).toBe("rate");
  expect(stepIcon("DSP: Bass +3 dB")).toBe("dsp");
  expect(stepIcon("Parametric EQ: 1 band: 100 Hz low shelf +3.0 dB")).toBe(
    "dsp",
  );
  expect(stepIcon("Headroom: -3.0 dB")).toBe("dsp");
  expect(stepIcon("Crossfeed: Medium")).toBe("dsp");
  expect(stepIcon("Convolution: Room correction")).toBe("dsp");
  expect(
    stepIcon("Sample rate conversion: 44.1kHz to 48kHz (linear phase)"),
  ).toBe("rate");
  expect(stepIcon("Bit depth reduction: 24bit to 16bit, TPDF dither")).toBe(
    "depth",
  );
  expect(
    stepIcon(
      "Renderer reports: decoding FLAC 96 kHz/24-bit, exactly what Parson sends",
    ),
  ).toBe("decode");
  expect(
    stepIcon("Sync: Living Room is the clock master and gets every sample"),
  ).toBe("sync");
});

test("caveats say what was tested, never more", () => {
  expect(signalPathCaveat({ kind: "sonos" })).toContain(
    "not yet on Sonos hardware",
  );
  expect(signalPathCaveat({ kind: "upnp" })).toContain("simulator");
  expect(signalPathCaveat({ kind: "bridge" })).toContain(
    "not yet on a USB DAC",
  );
  expect(signalPathCaveat({ kind: "airplay" })).toContain(
    "not yet on Apple hardware",
  );
  expect(signalPathCaveat({ kind: "web" })).toBeNull();
});
