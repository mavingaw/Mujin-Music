import type { BridgeStatus } from "@parson/music-sdk";
import { expect, test } from "bun:test";
import {
  addBridgeError,
  bridgeSummary,
  clockLabel,
  outputCounts,
} from "./zone-outputs-model";

const bridge = (overrides: Partial<BridgeStatus>): BridgeStatus => ({
  address: "10.0.0.7:1994",
  source: "discovered",
  connected: true,
  bridge_id: "pi",
  name: "kitchen-pi",
  host: "kitchen-pi",
  platform: "linux aarch64",
  version: "1.0.1",
  backend: "ALSA",
  devices: 2,
  clock_offset_us: 2_500_000,
  clock_uncertainty_us: 240,
  note: null,
  last_error: null,
  ...overrides,
});

test("bridge rows say what the bridge is and how well it keeps time", () => {
  expect(bridgeSummary(bridge({}))).toEqual({
    title: "kitchen-pi",
    detail: "linux aarch64 · 2 outputs · ALSA · clock synced ±0.2 ms",
    tone: "ok",
  });
  expect(
    bridgeSummary(
      bridge({ platform: "windows x86_64", backend: "WASAPI", devices: 1 }),
    ).detail,
  ).toBe("windows x86_64 · 1 output · WASAPI exclusive · clock synced ±0.2 ms");
  expect(
    bridgeSummary(bridge({ platform: "macos aarch64", backend: "CoreAudio" }))
      .detail,
  ).toBe(
    "macos aarch64 · 2 outputs · CoreAudio hog mode · clock synced ±0.2 ms",
  );
  expect(clockLabel(bridge({ clock_uncertainty_us: 20 }))).toBe(
    "clock synced ±0.1 ms",
  );
  expect(
    bridgeSummary(
      bridge({ connected: false, last_error: "refused", name: null }),
    ),
  ).toEqual({
    title: "10.0.0.7:1994",
    detail: "Not connected: refused",
    tone: "off",
  });
  expect(
    bridgeSummary(
      bridge({
        devices: 0,
        note: "this system has no supported audio API",
      }),
    ).tone,
  ).toBe("warn");
});

test("output counts cover bridges, AirPlay and Chromecast", () => {
  const counts = outputCounts({
    upnp: { enabled: true, renderers: 0, last_error: null },
    devices: { enabled: false, backend: null, devices: 0, note: null },
    bridges: {
      enabled: true,
      bridges: [bridge({}), bridge({ connected: false, devices: 4 })],
    },
    airplay: {
      enabled: true,
      note: null,
      receivers: [
        {
          id: "a",
          name: "Kitchen",
          model: null,
          address: "",
          supported: true,
          reason: null,
        },
        {
          id: "b",
          name: "HomePod",
          model: null,
          address: "",
          supported: false,
          reason: "AirPlay 2 only",
        },
      ],
    },
    chromecast: {
      enabled: true,
      note: null,
      devices: [
        { id: "c", name: "TV", model: "Chromecast", address: "", group: false },
      ],
    },
  });
  expect(counts).toEqual({
    bridges: 1,
    bridgeOutputs: 2,
    airplay: 1,
    airplayUnsupported: ["HomePod"],
    chromecast: 1,
  });
  expect(outputCounts(null).bridges).toBe(0);
  expect(addBridgeError({ response: { status: 403 } })).toContain(
    "administrator",
  );
  expect(
    addBridgeError({ response: { data: { error: "invalid_bridge_address" } } }),
  ).toContain("pi.local");
});
