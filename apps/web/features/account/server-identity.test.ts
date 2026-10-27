import { expect, test } from "bun:test";
import { readServerIdentity, signInHeading } from "./server-identity";

test("the sign-in page names the server and its address", () => {
  expect(
    signInHeading({
      serverName: "Basement NAS",
      requestedName: null,
      host: "192.168.1.20:1993",
    }),
  ).toEqual({ title: "Sign in to Basement NAS", address: "192.168.1.20:1993" });
  // The server's own name wins over one carried in a link.
  expect(
    signInHeading({
      serverName: "Study Mini",
      requestedName: "Basement NAS",
      host: "192.168.1.21:1993",
    }).title,
  ).toBe("Sign in to Study Mini");
  expect(
    signInHeading({ serverName: null, requestedName: " ", host: "x:1" }),
  ).toEqual({ title: "Sign in", address: "x:1" });
});

test("the server's name comes from its public manifest", async () => {
  const fetcher = (async (url: string) => {
    expect(url).toBe("/.well-known/parson");
    return new Response(
      JSON.stringify({ name: "Basement NAS", instanceId: "abc" }),
    );
  }) as unknown as typeof fetch;
  expect(await readServerIdentity(fetcher)).toEqual({
    name: "Basement NAS",
    instanceId: "abc",
  });
  const failing = (async () =>
    new Response("", { status: 503 })) as unknown as typeof fetch;
  await expect(readServerIdentity(failing)).rejects.toThrow("503");
});
