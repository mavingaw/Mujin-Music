import { expect, test } from "bun:test";
import type { Zone } from "@parson/music-sdk";
import {
  queuedOnZoneMessage,
  resolvePlaybackTarget,
  zoneActionError,
} from "./zone-playback-model";

function zone(id: string, name: string, overrides: Partial<Zone> = {}): Zone {
  return {
    id,
    name,
    capabilities: { transfer_target: true, queue_edit: true },
    queue: { song_ids: ["a"], current_position: 0 },
    ...overrides,
  } as unknown as Zone;
}

const local = zone("client:browser-1", "This browser");
const dac = zone("bridge:pi-id:usb-dac", "Listening Room DAC");
const kitchen = zone("sonos:RINCON_K", "Kitchen");

test("This device plays in the browser; any other zone gets the command", () => {
  const base = {
    localZoneId: local.id,
    zones: [local, dac, kitchen],
    loaded: true,
  };
  expect(resolvePlaybackTarget({ ...base, requestedZoneId: null })).toEqual({
    kind: "local",
  });
  expect(resolvePlaybackTarget({ ...base, requestedZoneId: local.id })).toEqual(
    { kind: "local" },
  );
  expect(resolvePlaybackTarget({ ...base, requestedZoneId: dac.id })).toEqual({
    kind: "zone",
    zoneId: dac.id,
    name: "Listening Room DAC",
    canLoad: true,
    canQueue: true,
  });
  expect(
    resolvePlaybackTarget({ ...base, requestedZoneId: kitchen.id }),
  ).toMatchObject({ kind: "zone", zoneId: kitchen.id, name: "Kitchen" });
});

test("a picked zone keeps its commands while the zone list loads", () => {
  expect(
    resolvePlaybackTarget({
      requestedZoneId: kitchen.id,
      localZoneId: null,
      zones: [],
      loaded: false,
    }),
  ).toMatchObject({ kind: "zone", zoneId: kitchen.id });
  // Once loaded, a zone that went away falls back to this device.
  expect(
    resolvePlaybackTarget({
      requestedZoneId: "sonos:GONE",
      localZoneId: local.id,
      zones: [local],
      loaded: true,
    }),
  ).toEqual({ kind: "local" });
});

test("cast sessions and fixed queues say what they cannot take", () => {
  const cast = zone("cast:session-1", "Living Room TV", {
    capabilities: { transfer_target: false, queue_edit: false },
  } as Partial<Zone>);
  expect(
    resolvePlaybackTarget({
      requestedZoneId: cast.id,
      localZoneId: local.id,
      zones: [local, cast],
      loaded: true,
    }),
  ).toMatchObject({ canLoad: false, canQueue: false });
  // An idle zone with nothing queued takes Add to queue (it starts there).
  const idle = zone("upnp:uuid-1", "Study", {
    capabilities: { transfer_target: true, queue_edit: false },
    queue: { song_ids: [], current_position: 0 },
  } as Partial<Zone>);
  expect(
    resolvePlaybackTarget({
      requestedZoneId: idle.id,
      localZoneId: local.id,
      zones: [idle],
      loaded: true,
    }),
  ).toMatchObject({ canQueue: true });
});

test("messages name the zone", () => {
  expect(queuedOnZoneMessage("Blue Train", "Kitchen", true)).toBe(
    "Blue Train will play next on Kitchen",
  );
  expect(queuedOnZoneMessage("Blue Train", "Kitchen", false)).toBe(
    "Added Blue Train to the queue on Kitchen",
  );
  expect(
    zoneActionError({ response: { data: { error: "queue_full" } } }, "Kitchen"),
  ).toBe("Kitchen's queue is full (500 songs).");
  expect(zoneActionError(new Error("boom"), "Kitchen")).toBe(
    "Kitchen did not take that (boom).",
  );
});
