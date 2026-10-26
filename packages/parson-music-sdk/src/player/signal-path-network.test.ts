import { describe, expect, test } from "bun:test";
import {
  buildSignalPath,
  type ClientSignalLeg,
  type ServerSignalPath,
  type SignalStage,
} from "./signal-path";

const client: ClientSignalLeg = {
  platform: "web",
  engine: "media-element",
  playbackRate: 1,
  preservePitch: true,
  reverbWet: 0,
  output: { kind: "native", name: "Den Streamer" },
};

const source: SignalStage = {
  id: "source",
  kind: "source",
  title: "Source",
  detail: "FLAC 96kHz 24bit 2ch",
  quality: "lossless",
  explanation: "",
};

function networkPath(
  protocol: "open_home" | "air_play",
  stages: SignalStage[],
  extra: Partial<ServerSignalPath> = {},
): ServerSignalPath {
  const format = {
    codec: "flac",
    container: "flac",
    sample_rate: 96_000,
    bit_depth: 24,
    channels: 2,
    bitrate_kbps: null,
    lossless: true,
    high_res: true,
    summary: "FLAC 96kHz 24bit 2ch",
  };
  return {
    song_id: "s",
    target: "native",
    mode: "native",
    quality: "lossless",
    source: format,
    delivered: format,
    content_type: "audio/pcm",
    requested_bitrate_kbps: 0,
    effect: null,
    stages: [source, ...stages],
    output_device: {
      name: "Den Streamer",
      host: "studio",
      transport: "OpenHome · WAV (LPCM) over HTTP",
      usb_id: null,
      usb_speed: null,
      simulated: false,
      network: {
        protocol,
        protocol_label: protocol === "air_play" ? "AirPlay" : "OpenHome",
        endpoint: "192.168.1.40:55178",
        manufacturer: "Linn",
        model: "Klimax DSM",
        kind: "streamer",
        caps_source: "assumed",
        stream_format: "WAV (LPCM) over HTTP",
        has_icon: true,
        verification: "Tested against protocol fakes.",
      },
    },
    ...extra,
  };
}

describe("network outputs in the signal path", () => {
  test("an OpenHome renderer is bit-perfect over its protocol and confirms it", () => {
    const path = buildSignalPath(
      networkPath(
        "open_home",
        [
          {
            id: "native-transport",
            kind: "transport",
            title: "OpenHome to Den Streamer",
            detail: "WAV (LPCM) over HTTP, studio → 192.168.1.40:55178",
            quality: "lossless",
            explanation: "",
          },
          {
            id: "native-output",
            kind: "output",
            title: "Den Streamer",
            detail: "Linn Klimax DSM, 96kHz 24bit 2ch",
            quality: "lossless",
            explanation: "",
          },
          {
            id: "native-hardware",
            kind: "output",
            title: "Confirmed by the renderer",
            detail: "decoding WAV 24bit, lossless at 96kHz",
            quality: "lossless",
            explanation: "",
          },
        ],
        { hardware_confirmed: true },
      ),
      client,
    );
    expect(path.quality).toBe("lossless");
    expect(path.bitPerfect).toBeTrue();
    expect(path.transport).toBe(
      "WAV (LPCM) over HTTP, studio → 192.168.1.40:55178",
    );
    expect(path.hardwareVerified).toBeTrue();
    expect(path.hardwareReporter).toBe("renderer");
    expect(path.outputDevice?.network?.endpoint).toBe("192.168.1.40:55178");
    expect(path.description).toBe(
      "A high-resolution lossless stream reaches Den Streamer bit-perfect, over OpenHome at 96kHz 24bit 2ch. Confirmed by the renderer: decoding WAV 24bit, lossless at 96kHz.",
    );
  });

  test("AirPlay is high quality and names its resample, not a shared mixer", () => {
    const path = buildSignalPath(
      networkPath("air_play", [
        {
          id: "native-resample",
          kind: "conversion",
          title: "Sample rate conversion",
          detail: "96kHz to 44.1kHz (soxr, very high quality)",
          quality: "high_quality",
          explanation: "",
        },
        {
          id: "native-transport",
          kind: "transport",
          title: "AirPlay to Kitchen",
          detail:
            "ALAC 44.1kHz 16bit over RTP (AirPlay 1), studio → 10.0.0.9:7000",
          quality: "high_quality",
          explanation: "",
        },
        {
          id: "native-output",
          kind: "output",
          title: "Kitchen",
          detail: "Apple HomePod mini, 44.1kHz 16bit 2ch",
          quality: "high_quality",
          explanation: "",
        },
      ]),
      { ...client, output: { kind: "native", name: "Kitchen" } },
    );
    expect(path.quality).toBe("high_quality");
    expect(path.description).toContain(
      "sample rate conversion 96kHz to 44.1kHz",
    );
    expect(path.description).toContain("Kitchen is an AirPlay receiver");
    expect(path.description).not.toContain("shared mode");
  });

  test("a lossy file the driver confirms is confirmed, not contradicted", () => {
    const mp3: SignalStage = {
      ...source,
      detail: "MP3 44.1kHz 2ch 320kbps",
      quality: "low_quality",
    };
    const server: ServerSignalPath = {
      ...networkPath("open_home", []),
      output_device: null,
      quality: "low_quality",
      stages: [
        mp3,
        {
          id: "native-output",
          kind: "output",
          title: "USB DAC",
          detail: "ALSA hw:1,0 exclusive, 44.1kHz 24bit 2ch",
          quality: "low_quality",
          explanation: "",
        },
        {
          id: "native-hardware",
          kind: "output",
          title: "Confirmed by the driver",
          detail: "hardware running at S24_3LE 44.1kHz 2ch",
          quality: "low_quality",
          explanation: "",
        },
      ],
      hardware_confirmed: true,
    };
    const path = buildSignalPath(server, client);
    expect(path.quality).toBe("low_quality");
    expect(path.hardwareVerified).toBeTrue();
    expect(path.hardwareReporter).toBe("driver");
    // Older servers without the flag: the stage title decides.
    const older = buildSignalPath(
      { ...server, hardware_confirmed: undefined },
      client,
    );
    expect(older.hardwareVerified).toBeTrue();
  });
});
