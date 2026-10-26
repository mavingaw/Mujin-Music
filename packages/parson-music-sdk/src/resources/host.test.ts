import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  connectLink,
  getHostInfo,
  hostAddressLabel,
  hostOrigins,
  hostUptime,
  parseConnectLink,
  parseHostInfo,
  usableRemoteOrigins,
} from "./host";

afterEach(() => mock.restore());

const raw = {
  instanceId: "a1b2c3",
  name: "Listening room",
  serverVersion: "1.0.1",
  port: 1993,
  bootId: "boot-1",
  startedAt: 1_700_000_000,
  networkAccess: true,
  advertising: true,
  publicUrl: null,
  addresses: [
    {
      address: "192.168.1.42",
      origin: "http://192.168.1.42:1993",
      interface: "wlan0",
      kind: "lan",
    },
    { address: "x", origin: "javascript:alert(1)", kind: "lan" },
    { address: "y", origin: "http://10.0.0.2:1993", kind: "bogus" },
  ],
};

test("host info is validated and drops unusable addresses", () => {
  const info = parseHostInfo(raw);
  expect(info?.name).toBe("Listening room");
  expect(info?.addresses.map((address) => address.origin)).toEqual([
    "http://192.168.1.42:1993",
  ]);
  expect(parseHostInfo({ ...raw, port: 0 })).toBeNull();
  expect(parseHostInfo({ ...raw, instanceId: "" })).toBeNull();
  expect(parseHostInfo(null)).toBeNull();
});

test("host info is read from the authenticated host endpoint", async () => {
  const get = spyOn(api, "get").mockResolvedValue({
    data: raw,
    status: 200,
    headers: new Headers(),
  } as never);
  expect((await getHostInfo())?.instanceId).toBe("a1b2c3");
  expect(get).toHaveBeenCalledWith("/host", { timeout: 5_000 });
});

test("a configured public URL is offered before network addresses", () => {
  const info = parseHostInfo({
    ...raw,
    publicUrl: "https://music.example/",
  })!;
  expect(hostOrigins(info)).toEqual([
    "https://music.example",
    "http://192.168.1.42:1993",
  ]);
  expect(hostAddressLabel(info.addresses[0]!)).toBe("Home network");
});

test("connect links round-trip through the parson scheme", () => {
  const info = parseHostInfo(raw)!;
  const link = connectLink(info, "http://192.168.1.42:1993", "app");
  expect(link.startsWith("parson://connect?")).toBe(true);
  expect(parseConnectLink(link)).toEqual({
    origin: "http://192.168.1.42:1993",
    name: "Listening room",
    instanceId: "a1b2c3",
    remote: [],
  });
  expect(connectLink(info, "http://192.168.1.42:1993", "browser")).toBe(
    "http://192.168.1.42:1993/",
  );
  expect(connectLink(info, "ftp://nope", "app")).toBe("");
});

test("connect links reject other schemes, routes and server addresses", () => {
  expect(parseConnectLink("https://example.com/connect?server=x")).toBeNull();
  expect(parseConnectLink("parson://album?server=http://a:1")).toBeNull();
  expect(
    parseConnectLink("parson://connect?server=file:///etc/passwd"),
  ).toBeNull();
  expect(
    parseConnectLink("parson:///connect?server=http%3A%2F%2F10.0.0.9%3A1993"),
  ).toEqual({
    origin: "http://10.0.0.9:1993",
    name: null,
    instanceId: null,
    remote: [],
  });
});

test("away-from-home addresses are validated and carried by connect links", () => {
  const info = parseHostInfo({
    ...raw,
    remote: [
      {
        origin: "https://music.example.com",
        kind: "manual",
        label: "Your address",
        verified: true,
      },
      {
        origin: "http://music-nas.tail1234.ts.net:1993",
        kind: "vpn",
        label: "Tailscale",
        verified: null,
      },
      {
        origin: "http://203.0.113.9:41993",
        kind: "router",
        label: "Port forward",
        verified: false,
      },
      { origin: "javascript:alert(1)", kind: "vpn", label: "x" },
      { origin: "http://10.9.9.9:1", kind: "teleport", label: "x" },
    ],
  })!;
  expect(info.remote.map((address) => address.kind)).toEqual([
    "manual",
    "vpn",
    "router",
  ]);
  // A path the server found unreachable is not handed to the phone.
  expect(usableRemoteOrigins(info)).toEqual([
    "https://music.example.com",
    "http://music-nas.tail1234.ts.net:1993",
  ]);
  const link = connectLink(info, "http://192.168.1.42:1993", "app");
  expect(parseConnectLink(link)?.remote).toEqual([
    "https://music.example.com",
    "http://music-nas.tail1234.ts.net:1993",
  ]);
  expect(
    parseConnectLink(
      "parson://connect?server=http://10.0.0.9:1993&remote=file:///x&remote=http://10.0.0.9:1993&remote=https://a.example",
    )?.remote,
  ).toEqual(["https://a.example"]);
  expect(parseHostInfo(raw)?.remote).toEqual([]);
});

test("uptime reads naturally", () => {
  const now = 1_700_000_000_000;
  expect(hostUptime(1_700_000_000 - 30, now)).toBe("less than a minute");
  expect(hostUptime(1_700_000_000 - 3 * 3_600, now)).toBe("3 hours");
  expect(hostUptime(1_700_000_000 - 86_400, now)).toBe("1 day");
  expect(hostUptime(0, now)).toBe("");
});
