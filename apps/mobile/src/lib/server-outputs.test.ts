import { expect, test } from "bun:test";
import {
  defaultDspSettings,
  type NativeOutputDevice,
  type NativeOutputSession,
} from "@parson/music-sdk";
import {
  networkPlayerLine,
  networkPlayerNote,
  serverDriverLine,
  serverDsdLine,
  serverHardwareLine,
  serverDspStatusLine,
  serverDspSummary,
  serverOutputSummary,
  serverSessionLine,
  serverTransportLine,
  unsupportedRates,
} from "./server-outputs";

const device = {
  id: "wasapi:{1}",
  name: "USB DAC",
  formats: {
    sample_rates: [44_100, 96_000, 192_000, 352_800].map((value) => ({
      value,
      supported: value <= 192_000,
      allowed: true,
    })),
    bit_depths: [16, 24, 32].map((value) => ({
      value,
      supported: value < 32,
      allowed: true,
    })),
  },
  probe_error: null,
} as unknown as NativeOutputDevice;

test("summaries list what the device plays bit-perfectly", () => {
  expect(serverOutputSummary(device)).toBe(
    "Bit-perfect 44.1kHz–192kHz, 16/24 bit",
  );
  expect(unsupportedRates(device)).toBe("352.8kHz");
  expect(
    serverOutputSummary({
      ...device,
      formats: null,
      probe_error: "in use",
    }),
  ).toBe("in use");
});

test("the session line shows state and position", () => {
  expect(
    serverSessionLine({
      state: "paused",
      position_ms: 83_400,
      duration_ms: 240_000,
    } as NativeOutputSession),
  ).toBe("Paused · 1:23 / 4:00");
  expect(
    serverSessionLine({
      state: "failed",
      error: "device busy",
      position_ms: 0,
      duration_ms: 0,
    } as NativeOutputSession),
  ).toBe("device busy · 0:00 / 0:00");
});

test("the server's DSP chain is summarised per device", () => {
  expect(serverDspSummary(device)).toBeNull();
  const dsp = defaultDspSettings();
  expect(serverDspSummary({ ...device, dsp })).toBeNull();
  dsp.headroom = { enabled: true, gainDb: -3 };
  dsp.eq.enabled = true;
  dsp.eq.bands[0]!.gainDb = 4;
  dsp.eq.bands[3]!.gainDb = -2;
  dsp.crossfeed.enabled = true;
  dsp.sampleRate = { enabled: true, mode: 96_000 };
  expect(serverDspSummary({ ...device, dsp })).toBe(
    "Headroom −3.0 dB · EQ 2 bands · Crossfeed Medium · 96kHz",
  );
  expect(
    serverDspSummary({ ...device, dsp: { ...dsp, enabled: false } }),
  ).toBeNull();
});

test("the live DSP status reads like Roon's processing speed", () => {
  expect(serverDspStatusLine(null)).toBeNull();
  expect(
    serverDspStatusLine({
      realtime_factor: 41.27,
      peak_dbfs: 1.4,
      clipped_samples: 12,
      clipping: true,
    }),
  ).toBe("Processing speed 41.3x · Peak +1.4 dBFS · Clipping (12)");
});

test("DSD support, hardware and the driver's report are summarised", () => {
  const dsdDevice = {
    ...device,
    settings: {
      exclusive: true,
      max_sample_rate: null,
      max_bit_depth: null,
      dsd_strategy: "dop",
      max_dsd_rate: null,
    },
    formats: {
      ...device.formats,
      dsd_rates: [2_822_400, 5_644_800].map((value) => ({
        value,
        supported: false,
        allowed: true,
      })),
      dop_rates: [2_822_400, 5_644_800],
    },
    hardware: {
      bus: "usb",
      usb_id: "21b4:0082",
      product: "AudioQuest DragonFly Red v1.0",
      speed: "high speed",
      caps_source: "driver",
      altsets: [],
    },
  } as unknown as NativeOutputDevice;
  expect(serverDsdLine(dsdDevice)).toBe(
    "DSD: DoP up to DSD128 · DSD over PCM (DoP v1.0)",
  );
  expect(serverHardwareLine(dsdDevice)).toBe("USB 21b4:0082 · high speed");
  const session = {
    hardware: {
      source: "/proc/asound/card1/pcm0p/sub0/hw_params",
      params: {
        access: "RW_INTERLEAVED",
        format: "S24_3LE",
        channels: 2,
        rate: 96_000,
        period_size: null,
        buffer_size: null,
      },
      momentary_hz: 96_000,
      matches_stream: true,
    },
  } as unknown as NativeOutputSession;
  expect(serverDriverLine(session)).toBe(
    "Confirmed by the driver: hardware running at S24_3LE 96kHz 2ch",
  );
  expect(serverDriverLine({} as NativeOutputSession)).toBeNull();
});

test("libasound reports name the plugin chain, and the transport names the driver", () => {
  const params = {
    access: "RW_INTERLEAVED",
    format: "S16_LE",
    channels: 2,
    rate: 44_100,
    period_size: 2_205,
    buffer_size: 8_820,
  };
  const confirmed = {
    hardware: {
      source: "snd_pcm_dump(parson_capture)",
      reporter: "library",
      chain: {
        plugins: ["File PCM", "Null PCM"],
        bit_exact: true,
        converting: [],
      },
      params,
      momentary_hz: null,
      matches_stream: true,
    },
    signal_path: {
      output_device: {
        name: "E30",
        host: "studio-pc",
        transport: "ALSA hw:1,0 exclusive",
        usb_id: "152a:8750",
        usb_speed: "high speed",
        simulated: false,
      },
    },
  } as unknown as NativeOutputSession;
  expect(serverDriverLine(confirmed)).toBe(
    "Confirmed by the driver: running at S16_LE 44.1kHz 2ch via File PCM → Null PCM",
  );
  expect(serverTransportLine(confirmed)).toBe(
    "USB 152a:8750 · high speed · ALSA hw:1,0 exclusive on studio-pc",
  );
  const converted = {
    hardware: {
      ...confirmed.hardware,
      chain: {
        plugins: ["Linear conversion PCM", "Null PCM"],
        bit_exact: false,
        converting: ["Linear conversion PCM"],
      },
      params: { ...params, format: "S32_LE" },
      matches_stream: false,
    },
  } as unknown as NativeOutputSession;
  expect(serverDriverLine(converted)).toBe(
    "Driver: S32_LE 44.1kHz 2ch, converted after Parson by Linear conversion PCM",
  );
  expect(serverTransportLine({} as NativeOutputSession)).toBeNull();
  const bench = {
    signal_path: {
      output_device: {
        name: "Parson Bench DAC (kernel ABI emulator)",
        host: "studio-pc",
        transport: "ALSA hw:1,0 exclusive",
        usb_id: "1209:0001",
        usb_speed: "high speed",
        simulated: false,
        kernel_emulator: true,
      },
    },
  } as unknown as NativeOutputSession;
  expect(serverTransportLine(bench)).toBe(
    "USB 1209:0001 · high speed · ALSA hw:1,0 exclusive on studio-pc · kernel ABI emulator, no DAC hardware",
  );
});

test("network players name their protocol, limits and test status", () => {
  const upnp = {
    ...device,
    id: "upnp:abc",
    name: "Den Streamer",
    driver: "upnp",
    formats: {
      sample_rates: [44_100, 96_000, 192_000, 352_800].map((value) => ({
        value,
        supported: value <= 192_000,
        allowed: true,
      })),
      bit_depths: [16, 24, 32].map((value) => ({
        value,
        supported: value < 32,
        allowed: true,
      })),
    },
    network: {
      protocol: "open_home",
      host: "192.168.1.40",
      port: 55178,
      manufacturer: "Linn",
      model: "Klimax DSM",
      caps_source: "assumed",
    },
  } as unknown as NativeOutputDevice;
  expect(serverOutputSummary(upnp)).toBe("44.1kHz–192kHz, 16/24 bit (assumed)");
  expect(networkPlayerLine(upnp)).toBe(
    "Linn Klimax DSM · OpenHome · 192.168.1.40",
  );
  expect(networkPlayerNote(upnp)).toContain("not yet a real streamer");
  const session = {
    hardware: {
      reporter: "renderer",
      params: { format: "WAV 24bit, lossless", rate: 96_000, channels: 2 },
      matches_stream: true,
    },
    signal_path: {
      output_device: {
        host: "studio",
        network: {
          protocol_label: "OpenHome",
          endpoint: "192.168.1.40:55178",
          stream_format: "WAV (LPCM) over HTTP",
        },
      },
    },
  } as unknown as NativeOutputSession;
  expect(serverDriverLine(session)).toBe(
    "Confirmed by the renderer: decoding WAV 24bit, lossless at 96kHz",
  );
  expect(serverTransportLine(session)).toBe(
    "OpenHome to 192.168.1.40:55178 · WAV (LPCM) over HTTP from studio",
  );
  const dsp = defaultDspSettings();
  dsp.convolution = { enabled: true, filterId: "ab12" };
  expect(serverDspSummary({ ...device, dsp } as NativeOutputDevice)).toBe(
    "Convolution",
  );
});
