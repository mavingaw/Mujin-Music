import { expect, test } from "bun:test";
import { parseHostInfo, type HostInfo } from "@parson/music-sdk";
import {
  hostReachability,
  isLoopbackOrigin,
  primaryHostOrigin,
  qrPath,
} from "./host-presentation";

const info = parseHostInfo({
  instanceId: "a",
  name: "Studio",
  port: 1993,
  networkAccess: true,
  advertising: true,
  addresses: [
    {
      address: "192.168.1.42",
      origin: "http://192.168.1.42:1993",
      interface: "wlan0",
      kind: "lan",
    },
    {
      address: "100.101.1.2",
      origin: "http://100.101.1.2:1993",
      interface: "tailscale0",
      kind: "vpn",
    },
  ],
}) as HostInfo;

test("loopback origins are recognised", () => {
  expect(isLoopbackOrigin("http://127.0.0.1:1993")).toBe(true);
  expect(isLoopbackOrigin("http://localhost:3000")).toBe(true);
  expect(isLoopbackOrigin("http://192.168.1.42:1993")).toBe(false);
});

test("the QR code uses an address phones can reach", () => {
  // Desktop shows the app from 127.0.0.1: use the LAN address instead.
  expect(primaryHostOrigin(info, "http://127.0.0.1:1993")).toBe(
    "http://192.168.1.42:1993",
  );
  // Opened through the VPN address: that one demonstrably works.
  expect(primaryHostOrigin(info, "http://100.101.1.2:1993")).toBe(
    "http://100.101.1.2:1993",
  );
  // Opened through a name (reverse proxy, mDNS hostname): keep it.
  expect(primaryHostOrigin(info, "https://music.example")).toBe(
    "https://music.example",
  );
});

test("reachability explains why other devices cannot connect", () => {
  expect(hostReachability(info)).toEqual({ kind: "ready" });
  expect(hostReachability({ ...info, networkAccess: false }).kind).toBe(
    "loopback-only",
  );
  expect(hostReachability({ ...info, addresses: [] }).kind).toBe("no-network");
  expect(hostReachability({ ...info, advertising: false }).kind).toBe(
    "not-advertising",
  );
});

test("QR matrices become compact SVG paths", () => {
  // 3x3: top row dark, centre dark.
  const matrix = [1, 1, 1, 0, 1, 0, 0, 0, 0];
  expect(qrPath(matrix)).toEqual({
    size: 3,
    path: "M0 0h3v1h-3zM1 1h1v1h-1z",
  });
  expect(qrPath([1, 0]).size).toBe(0);
});
