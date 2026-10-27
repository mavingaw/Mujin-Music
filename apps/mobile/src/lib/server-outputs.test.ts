import { expect, test } from "bun:test";
import {
  defaultDspSettings,
  type SignalPath,
  type Zone,
  type ZoneEndpoint,
} from "@parson/music-sdk";
import {
  networkPlayerLine,
  networkPlayerNote,
  serverDriverLine,
  serverDsdLine,
  serverHardwareLine,
  serverDspStatusLine,
  serverDspSummary,
  serverOutputs,
  serverOutputSummary,
  serverSessionLine,
  serverTransportLine,
  unsupportedRates,
} from "./server-outputs";

const device = {
  id: "device:wasapi:{1}",
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
  network: null,
  hardware: null,
} as unknown as ZoneEndpoint;

test("only zones with an endpoint of their own are server outputs", () => {
  const zone = (id: string, kind: string, endpoints: ZoneEndpoint[]) =>
    ({ id, kind, endpoints }) as unknown as Zone;
  expect(
    serverOutputs([
      zone("phone", "mobile", [{ ...device, id: "phone" }]),
      zone("device:wasapi:{1}", "device", [device]),
      zone("group", "linked", [device]),
    ]).map((output) => output.zone.id),
  ).toEqual(["device:wasapi:{1}"]);
});

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
      status: "paused",
      position_ms: 83_400,
      duration_ms: 240_000,
    }),
  ).toBe("Paused · 1:23 / 4:00");
  expect(
    serverSessionLine({ status: "idle", position_ms: 0, duration_ms: 0 }),
  ).toBe("Stopped · 0:00 / 0:00");
});

test("a zone's DSP chain is summarised", () => {
  expect(serverDspSummary(null)).toBeNull();
  const dsp = defaultDspSettings();
  expect(serverDspSummary(dsp)).toBeNull();
  dsp.headroom = { enabled: true, gainDb: -3 };
  dsp.eq.enabled = true;
  dsp.eq.bands[0]!.gainDb = 4;
  dsp.eq.bands[3]!.gainDb = -2;
  dsp.crossfeed.enabled = true;
  dsp.sampleRate = { enabled: true, mode: 96_000 };
  expect(serverDspSummary(dsp)).toBe(
    "Headroom −3.0 dB · EQ 2 bands · Crossfeed Medium · 96kHz",
  );
  expect(serverDspSummary({ ...dsp, enabled: false })).toBeNull();
  const convolution = defaultDspSettings();
  convolution.convolution = { enabled: true, filterId: "ab12" };
  expect(serverDspSummary(convolution)).toBe("Convolution");
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
  } as unknown as ZoneEndpoint;
  expect(serverDsdLine(dsdDevice, "dop")).toBe(
    "DSD: DoP up to DSD128 · DSD over PCM (DoP v1.0)",
  );
  expect(serverDsdLine(device, "auto")).toBeNull();
  expect(serverHardwareLine(dsdDevice)).toBe("USB 21b4:0082 · high speed");
  expect(
    serverDriverLine({
      hardwareReport: "S24_3LE 96kHz 2ch",
      hardwareVerified: true,
      hardwareReporter: "driver",
    }),
  ).toBe("Confirmed by the driver: S24_3LE 96kHz 2ch");
  expect(
    serverDriverLine({
      hardwareReport: "S32_LE 44.1kHz 2ch",
      hardwareVerified: false,
      hardwareReporter: "driver",
    }),
  ).toBe("Driver: S32_LE 44.1kHz 2ch, converted after Mujin Music");
  expect(
    serverDriverLine({
      hardwareReport: null,
      hardwareVerified: null,
      hardwareReporter: null,
    }),
  ).toBeNull();
});

test("the transport names the driver, or the network hop", () => {
  const local = {
    outputDevice: {
      name: "E30",
      host: "studio-pc",
      transport: "ALSA hw:1,0 exclusive",
      usb_id: "152a:8750",
      usb_speed: "high speed",
      simulated: false,
    },
  } as unknown as SignalPath;
  expect(serverTransportLine(local)).toBe(
    "USB 152a:8750 · high speed · ALSA hw:1,0 exclusive on studio-pc",
  );
  expect(serverTransportLine({ outputDevice: null })).toBeNull();
  const bench = {
    outputDevice: { ...local.outputDevice!, kernel_emulator: true },
  } as SignalPath;
  expect(serverTransportLine(bench)).toBe(
    "USB 152a:8750 · high speed · ALSA hw:1,0 exclusive on studio-pc · kernel ABI emulator, no DAC hardware",
  );
  const network = {
    outputDevice: {
      host: "studio",
      network: {
        protocol_label: "OpenHome",
        endpoint: "192.168.1.40:55178",
        stream_format: "FLAC over HTTP",
      },
    },
  } as unknown as SignalPath;
  expect(serverTransportLine(network)).toBe(
    "OpenHome to 192.168.1.40:55178 · FLAC over HTTP from studio",
  );
});

test("network players name their protocol, limits and test status", () => {
  const upnp = {
    ...device,
    id: "upnp:abc",
    name: "Den Streamer",
    network: {
      protocol: "open_home",
      host: "192.168.1.40",
      port: 55178,
      manufacturer: "Linn",
      model: "Klimax DSM",
      caps_source: "assumed",
    },
  } as unknown as ZoneEndpoint;
  expect(serverOutputSummary(upnp)).toBe("44.1kHz–192kHz, 16/24 bit (assumed)");
  expect(networkPlayerLine(upnp)).toBe(
    "Linn Klimax DSM · OpenHome · 192.168.1.40",
  );
  expect(networkPlayerNote(upnp)).toContain("not yet a real streamer");
});
