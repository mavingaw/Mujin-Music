import { expect, test } from "bun:test";
import { checkRoute, routeCheckText, routeLabel } from "./route-check";

function fetcherFor(body: unknown, status = 200) {
  return (async () =>
    new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;
}

test("a route works only when it reaches the same library", async () => {
  let clock = 1_000;
  const now = () => (clock += 21);
  expect(
    await checkRoute("https://music.example.com", "home", {
      fetcher: fetcherFor({ protocol: "parson", instanceId: "home" }),
      now,
    }),
  ).toEqual({ ok: true, latencyMs: 21 });
  expect(
    await checkRoute("https://music.example.com", "home", {
      fetcher: fetcherFor({ protocol: "parson", instanceId: "studio" }),
    }),
  ).toEqual({ ok: false, reason: "other-library" });
  expect(
    await checkRoute("https://music.example.com", "home", {
      fetcher: fetcherFor({ hello: "router login" }),
    }),
  ).toEqual({ ok: false, reason: "not-parson" });
  expect(
    await checkRoute("https://music.example.com", "home", {
      fetcher: (async () => {
        throw new Error("offline");
      }) as unknown as typeof fetch,
    }),
  ).toEqual({ ok: false, reason: "unreachable" });
});

test("routes are labelled the way people know them", () => {
  expect(routeLabel("http://music-nas.tail1234.ts.net:1993")).toBe("Tailscale");
  expect(routeLabel("http://100.101.102.103:1993")).toBe("Tailscale");
  expect(routeLabel("https://music.example.com")).toBe("music.example.com");
  expect(routeCheckText({ ok: true, latencyMs: 40 })).toBe(
    "Works from here · 40 ms",
  );
  // From the home network a check can't show the way in from outside.
  expect(
    routeCheckText({ ok: true, latencyMs: 0 }, { onHomeNetwork: true }),
  ).toBe("Answers here at home · check again on mobile data");
  expect(routeCheckText({ ok: false, reason: "unreachable" })).toBe(
    "No answer from here",
  );
});
