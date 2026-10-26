import { expect, test } from "bun:test";
import type { Zone, ZoneEndpoint } from "@parson/music-sdk";
import {
  networkLimitsNote,
  networkStreamNote,
  outputEntries,
} from "./audio-outputs-model";

function endpoint(id: string, network: unknown = null): ZoneEndpoint {
  return { id, name: id, network } as unknown as ZoneEndpoint;
}

function zone(id: string, kind: Zone["kind"], ep?: ZoneEndpoint): Zone {
  return { id, kind, endpoints: ep ? [ep] : [] } as unknown as Zone;
}

const usb = endpoint("device:alsa:USB:0");

const streamer = endpoint("upnp:abc", {
  protocol: "open_home",
  caps_source: "assumed",
  stream_format: "WAV (LPCM) over HTTP",
  reports_format: true,
  model: "Klimax DSM",
});

const airplay = endpoint("airplay:aabb", {
  protocol: "air_play",
  caps_source: "protocol",
  stream_format: "ALAC 44.1kHz 16bit over RTP (AirPlay 1)",
  reports_format: false,
});

test("network players are listed apart from the server's own devices", () => {
  const { local, network } = outputEntries([
    zone("upnp:abc", "upnp", streamer),
    zone("web:1", "web"),
    zone("device:alsa:USB:0", "device", usb),
    zone("airplay:aabb", "airplay", airplay),
    // A group plays through its members' endpoints, listed under them.
    {
      ...zone("group:1", "linked", usb),
      group: { members: [] },
    } as unknown as Zone,
  ]);
  expect(local.map((entry) => entry.zone.id)).toEqual(["device:alsa:USB:0"]);
  expect(network.map((entry) => entry.zone.id)).toEqual([
    "upnp:abc",
    "airplay:aabb",
  ]);
});

test("each player says where its limits come from and how audio travels", () => {
  expect(networkLimitsNote(streamer)).toContain("does not list rates");
  expect(networkLimitsNote(airplay)).toContain("44.1kHz/16bit only");
  expect(networkStreamNote(streamer)).toContain("DSP included");
  expect(networkStreamNote(streamer)).toContain("reports what it decodes");
  expect(networkStreamNote(airplay)).toContain("High quality at best");
  expect(networkLimitsNote(usb)).toBeNull();
});
