import { expect, test } from "bun:test";
import type { RouterState } from "@parson/music-sdk";
import { describeRouter, outcomeLabel } from "./remote-access-presentation";

const router: RouterState = {
  status: "off",
  router_name: null,
  external_ip: null,
  external_port: null,
  internal_address: null,
  lease_seconds: null,
  updated_at: null,
  message: null,
};

test("router states read plainly and stay honest", () => {
  expect(describeRouter(router).tone).toBe("off");
  const forwarded = describeRouter({
    ...router,
    status: "forwarded",
    router_name: "FRITZ!Box 7590",
    external_ip: "203.0.113.9",
    external_port: 41993,
    internal_address: "192.168.1.20:1993",
    lease_seconds: 3600,
  });
  expect(forwarded.tone).toBe("good");
  expect(forwarded.title).toBe("Port 41993 forwarded by FRITZ!Box 7590");
  expect(forwarded.detail).toContain("203.0.113.9:41993");
  expect(forwarded.detail).toContain("renews");
  expect(
    describeRouter({ ...router, status: "forwarded", lease_seconds: 0 }).detail,
  ).toContain("permanent");
  const cgnat = describeRouter({
    ...router,
    status: "unreachable",
    message: "carrier-grade NAT",
  });
  expect(cgnat.tone).toBe("warn");
  expect(cgnat.detail).toBe("carrier-grade NAT");
  expect(describeRouter({ ...router, status: "failed" }).detail).toContain(
    "UPnP",
  );
});

test("check badges show latency only for reachable addresses", () => {
  const base = {
    origin: "http://x",
    kind: "vpn" as const,
    label: "Tailscale",
    detail: "",
  };
  expect(outcomeLabel({ ...base, outcome: "reachable", latency_ms: 42 })).toBe(
    "42 ms",
  );
  expect(
    outcomeLabel({ ...base, outcome: "unreachable", latency_ms: null }),
  ).toBe("No answer");
  expect(
    outcomeLabel({ ...base, outcome: "other_server", latency_ms: null }),
  ).toBe("Other server");
});
