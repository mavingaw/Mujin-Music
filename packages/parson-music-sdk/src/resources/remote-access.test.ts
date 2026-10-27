import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  checkRemoteAccess,
  getRemoteAccess,
  summarizeRemoteAccess,
  updateRemoteAccess,
  type RemoteAccessStatus,
} from "./remote-access";

afterEach(() => mock.restore());

const base: RemoteAccessStatus = {
  config: { mode: "off", manual_url: null, external_port: null },
  router: {
    status: "off",
    router_name: null,
    external_ip: null,
    external_port: null,
    internal_address: null,
    lease_seconds: null,
    updated_at: null,
    message: null,
  },
  addresses: [],
  tailscale_name: null,
  last_check: null,
  network_access: true,
};

test("remote access uses the admin endpoints", async () => {
  const response = { data: base, status: 200, headers: new Headers() };
  const get = spyOn(api, "get").mockResolvedValue(response as never);
  const put = spyOn(api, "put").mockResolvedValue(response as never);
  const post = spyOn(api, "post").mockResolvedValue({
    ...response,
    data: { checked_at: "now", results: [] },
  } as never);
  await getRemoteAccess();
  await updateRemoteAccess({ mode: "router", external_port: 41993 });
  await checkRemoteAccess();
  expect(get).toHaveBeenCalledWith("/remote-access");
  expect(put).toHaveBeenCalledWith(
    "/remote-access",
    { mode: "router", external_port: 41993 },
    { timeout: 30_000 },
  );
  expect(post).toHaveBeenCalledWith("/remote-access/check", undefined, {
    timeout: 30_000,
  });
});

test("the summary is honest about what was checked", () => {
  expect(summarizeRemoteAccess(base).title).toBe("Home network only");
  const failed = summarizeRemoteAccess({
    ...base,
    config: { ...base.config, mode: "router" },
    router: {
      ...base.router,
      status: "unreachable",
      message: "carrier-grade NAT",
    },
  });
  expect(failed.tone).toBe("warn");
  expect(failed.detail).toContain("carrier-grade NAT");

  const address = {
    origin: "http://nas.tail.ts.net:1993",
    kind: "vpn" as const,
    label: "Tailscale",
    verified: null,
  };
  expect(summarizeRemoteAccess({ ...base, addresses: [address] }).title).toBe(
    "Not checked yet",
  );
  const good = summarizeRemoteAccess({
    ...base,
    addresses: [{ ...address, verified: true }],
    last_check: {
      checked_at: "now",
      results: [
        {
          origin: address.origin,
          kind: "vpn",
          label: "Tailscale",
          outcome: "reachable",
          latency_ms: 38,
          detail: "Reaches this server.",
        },
      ],
    },
  });
  expect(good.tone).toBe("good");
  expect(good.title).toBe("Reachable over your VPN");
  expect(good.detail).toContain("same VPN");
  const bad = summarizeRemoteAccess({
    ...base,
    addresses: [{ ...address, verified: false }],
    last_check: {
      checked_at: "now",
      results: [
        {
          origin: address.origin,
          kind: "vpn",
          label: "Tailscale",
          outcome: "unreachable",
          latency_ms: null,
          detail: "No answer within 5 seconds.",
        },
      ],
    },
  });
  expect(bad.title).toBe("No address passed the check");
  expect(bad.detail).toBe("No answer within 5 seconds.");
});

test("a check from the server never reads as reachable away from home", () => {
  const manual = {
    origin: "https://music.example.com",
    kind: "manual" as const,
    label: "Your address",
    verified: true,
  };
  const leads = summarizeRemoteAccess({
    ...base,
    config: { ...base.config, mode: "manual", manual_url: manual.origin },
    addresses: [manual],
    last_check: {
      checked_at: "now",
      results: [
        {
          origin: manual.origin,
          kind: "manual",
          label: manual.label,
          outcome: "reachable",
          latency_ms: 0,
          detail: "Leads to this server when called from the server itself.",
          checked_from: "server",
        },
      ],
    },
  });
  expect(leads.tone).toBe("warn");
  expect(leads.title).not.toContain("Reachable away from home");
  expect(leads.detail).toContain("can’t test from outside");
  expect(leads.detail).not.toContain("0 ms");

  const lan = { ...manual, origin: "http://192.0.2.2:2014", verified: false };
  const homeOnly = summarizeRemoteAccess({
    ...base,
    addresses: [lan],
    last_check: {
      checked_at: "now",
      results: [
        {
          origin: lan.origin,
          kind: "manual",
          label: lan.label,
          outcome: "home_only",
          latency_ms: null,
          detail:
            "192.0.2.2 is reserved for documentation and testing, not the internet. It only works on your home network, not away from home.",
          checked_from: "server",
        },
      ],
    },
  });
  expect(homeOnly.title).toBe("Only works on your home network");
  expect(homeOnly.tone).toBe("warn");
});
