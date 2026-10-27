import { describe, expect, test } from "bun:test";
import { dsdRateLabel, DSD_STRATEGIES } from "../resources/outputs";
import {
  buildSignalPath,
  describeAudioFormat,
  describeCollectionFormat,
  formatDsdRate,
  outputFormatFromDetail,
  type ClientSignalLeg,
  type ServerSignalPath,
  type SignalStage,
} from "./signal-path";

const dsf = {
  codec: "dsd",
  container: "dsf",
  sample_rate: 2_822_400,
  bit_depth: 1,
  channels: 2,
  bitrate_kbps: 5_644,
  lossless: true,
};

const client: ClientSignalLeg = {
  platform: "web",
  engine: "web-audio",
  playbackRate: 1,
  preservePitch: true,
  reverbWet: 0,
  volume: 1,
  output: { kind: "native", name: "Topping D10s" },
};

function stage(partial: Partial<SignalStage> & { id: string }): SignalStage {
  return {
    kind: "conversion",
    title: partial.id,
    detail: "",
    quality: "lossless",
    explanation: "",
    ...partial,
  };
}

function nativeServer(stages: SignalStage[]): ServerSignalPath {
  const source = describeAudioFormat(dsf)!;
  return {
    song_id: "s",
    target: "native",
    mode: "native",
    quality: "lossless",
    source,
    delivered: source,
    content_type: "audio/pcm",
    requested_bitrate_kbps: 0,
    effect: null,
    stages,
  };
}

describe("DSD in the signal path", () => {
  test("DSD files are labelled by container and DSD rate, and count as hi-res", () => {
    const format = describeAudioFormat(dsf)!;
    expect(format.summary).toBe("DSF DSD64 2.8MHz 2ch");
    expect(format.high_res).toBeTrue();
    expect(formatDsdRate(5_644_800)).toBe("DSD128 5.6MHz");
    expect(dsdRateLabel(11_289_600)).toBe("DSD256");
    expect(
      describeAudioFormat({ ...dsf, container: "dff", sample_rate: 5_644_800 })
        ?.summary,
    ).toBe("DSDIFF DSD128 5.6MHz 2ch");
    const album = describeCollectionFormat([
      dsf,
      { ...dsf, sample_rate: 5_644_800 },
    ])!;
    expect(album.summary).toBe("DSF DSD64–DSD128");
    expect(
      describeCollectionFormat([dsf, { ...dsf, container: "dff" }])?.summary,
    ).toBe("DSF / DSDIFF DSD64");
    expect(album.quality).toBe("lossless");
    expect(album.highRes).toBeTrue();
  });

  test("native DSD to an exclusive output is bit-perfect and names the format", () => {
    const path = buildSignalPath(
      nativeServer([
        stage({ id: "source", kind: "source", detail: "DSF DSD64 2.8MHz 2ch" }),
        stage({ id: "native-dsd-read", kind: "decode" }),
        stage({
          id: "native-output",
          kind: "output",
          title: "Topping D10s",
          detail:
            "ALSA hw, exclusive, native DSD64 (DSD_U32_BE, 88.2kHz frames) 2ch",
        }),
      ]),
      client,
    );
    expect(path.quality).toBe("lossless");
    expect(path.bitPerfect).toBeTrue();
    expect(path.hardwareVerified).toBeNull();
    expect(path.description).toBe(
      "A high-resolution lossless stream reaches Topping D10s bit-perfect, in exclusive mode at native DSD64 (DSD_U32_BE, 88.2kHz frames) 2ch.",
    );
  });

  test("the driver's report confirms or contradicts the path", () => {
    const base = [
      stage({ id: "source", kind: "source" }),
      stage({ id: "native-decode", kind: "decode" }),
      stage({
        id: "native-output",
        kind: "output",
        title: "DragonFly Red",
        detail: "ALSA hw, exclusive, 96kHz 24bit 2ch",
      }),
    ];
    const verified = buildSignalPath(
      nativeServer([
        ...base,
        stage({
          id: "native-hardware",
          kind: "output",
          title: "Confirmed by the driver",
          detail: "hardware running at S24_3LE 96kHz 2ch",
        }),
      ]),
      client,
    );
    expect(verified.hardwareVerified).toBeTrue();
    expect(verified.hardwareReport).toBe(
      "hardware running at S24_3LE 96kHz 2ch",
    );
    expect(verified.bitPerfect).toBeTrue();
    expect(verified.description).toContain(
      "Confirmed by the driver: hardware running at S24_3LE 96kHz 2ch.",
    );
    const converted = buildSignalPath(
      nativeServer([
        ...base,
        stage({
          id: "native-hardware",
          kind: "output",
          title: "Driver report",
          detail: "S16_LE at 48kHz 2ch",
          quality: "high_quality",
        }),
      ]),
      client,
    );
    expect(converted.hardwareVerified).toBeFalse();
    expect(converted.quality).toBe("high_quality");
    expect(converted.bitPerfect).toBeFalse();
  });

  test("an ALSA hw path names its transport and ends with the driver's confirmation", () => {
    const path = buildSignalPath(
      nativeServer([
        stage({ id: "source", kind: "source" }),
        stage({ id: "native-decode", kind: "decode" }),
        stage({
          id: "native-transport",
          kind: "transport",
          title: "studio-pc · Mujin Music audio transport",
          detail: "ALSA hw:1,0 exclusive",
        }),
        stage({
          id: "native-output",
          kind: "output",
          title: "E30",
          detail: "ALSA hw:1,0 exclusive, 96kHz 24bit 2ch",
        }),
        stage({
          id: "native-hardware",
          kind: "output",
          title: "Confirmed by the driver",
          detail: "hardware running at S24_3LE 96kHz 2ch",
        }),
      ]),
      { ...client, output: { kind: "native", name: "E30" } },
    );
    expect(path.transport).toBe("ALSA hw:1,0 exclusive");
    expect(path.bitPerfect).toBeTrue();
    expect(path.hardwareVerified).toBeTrue();
    expect(path.description).toBe(
      "A high-resolution lossless stream reaches E30 bit-perfect, in exclusive mode at 96kHz 24bit 2ch. Confirmed by the driver: hardware running at S24_3LE 96kHz 2ch.",
    );
    expect(
      outputFormatFromDetail(
        "ALSA PCM studio, no conversions, 44.1kHz 16bit 2ch",
      ),
    ).toBe("44.1kHz 16bit 2ch");
  });

  test("output formats are read from every detail shape", () => {
    expect(
      outputFormatFromDetail("WASAPI Exclusive Mode, 96kHz 24bit 2ch"),
    ).toBe("96kHz 24bit 2ch");
    expect(
      outputFormatFromDetail(
        "ALSA hw, exclusive, DoP: DSD64 in 176.4kHz 24bit 2ch",
      ),
    ).toBe("DoP: DSD64 in 176.4kHz 24bit 2ch");
    expect(
      outputFormatFromDetail("ALSA hw, exclusive, native DSD128 2ch"),
    ).toBe("native DSD128 2ch");
    // "native DSD" needs a rate; the earliest DSD description wins.
    expect(outputFormatFromDetail("native DSD, 44.1kHz 16bit 2ch")).toBe(
      "44.1kHz 16bit 2ch",
    );
    expect(outputFormatFromDetail("DoP: DSD64, native DSD64")).toBe(
      "DoP: DSD64, native DSD64",
    );
  });

  test("long adversarial output details are read in linear time", () => {
    const started = performance.now();
    expect(outputFormatFromDetail(`${"native DSD9".repeat(50_000)}x`)).toBe(
      `${"native DSD9".repeat(50_000)}x`,
    );
    // A DSD description must run to the end of the last line.
    expect(outputFormatFromDetail(`${"DoP: ".repeat(50_000)}\n`)).toBe(
      `${"DoP: ".repeat(50_000)}\n`,
    );
    expect(outputFormatFromDetail("native DSD".repeat(50_000))).toBe(
      "native DSD".repeat(50_000),
    );
    expect(performance.now() - started).toBeLessThan(1_000);
  });

  test("every DSD strategy is offered with an explanation", () => {
    expect(DSD_STRATEGIES.map((strategy) => strategy.value)).toEqual([
      "auto",
      "native",
      "dop",
      "pcm",
    ]);
    expect(
      DSD_STRATEGIES.every((strategy) => strategy.description.length > 20),
    ).toBeTrue();
  });
});
