import { expect, test } from "bun:test";
import type { NativeOutputDevice } from "@parson/music-sdk";
import {
  driverLabels,
  isNetworkDevice,
  networkLimitsNote,
  networkStreamNote,
  splitOutputDevices,
} from "./audio-outputs-model";

const usb = {
  id: "alsa:USB:0",
  name: "USB DAC",
  driver: "alsa",
} as unknown as NativeOutputDevice;

const streamer = {
  id: "upnp:abc",
  name: "Den Streamer",
  driver: "upnp",
  network: {
    protocol: "open_home",
    caps_source: "assumed",
    stream_format: "WAV (LPCM) over HTTP",
    reports_format: true,
    model: "Klimax DSM",
  },
} as unknown as NativeOutputDevice;

const airplay = {
  id: "airplay:AABB",
  name: "Kitchen",
  driver: "airplay",
  network: {
    protocol: "air_play",
    caps_source: "protocol",
    stream_format: "ALAC 44.1kHz 16bit over RTP (AirPlay 1)",
    reports_format: false,
  },
} as unknown as NativeOutputDevice;

test("network players are listed apart from the server's own devices", () => {
  expect(isNetworkDevice(usb)).toBeFalse();
  expect(isNetworkDevice(streamer)).toBeTrue();
  const { local, network } = splitOutputDevices([streamer, usb, airplay]);
  expect(local.map((device) => device.id)).toEqual(["alsa:USB:0"]);
  expect(network.map((device) => device.id)).toEqual([
    "upnp:abc",
    "airplay:AABB",
  ]);
  expect(driverLabels.upnp).toBe("UPnP");
  expect(driverLabels.airplay).toBe("AirPlay");
});

test("each player says where its limits come from and how audio travels", () => {
  expect(networkLimitsNote(streamer)).toContain("does not list rates");
  expect(networkLimitsNote(airplay)).toContain("44.1kHz/16bit only");
  expect(networkStreamNote(streamer)).toContain("DSP included");
  expect(networkStreamNote(streamer)).toContain("reports what it decodes");
  expect(networkStreamNote(airplay)).toContain("High quality at best");
  expect(networkLimitsNote(usb)).toBeNull();
});
