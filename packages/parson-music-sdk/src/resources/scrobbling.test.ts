import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  authorizeLastfm,
  completeLastfm,
  connectListenbrainz,
  disconnectScrobbling,
  getScrobbling,
  importScrobbleHistory,
  plausibleLastfmCredential,
  plausibleListenbrainzToken,
  retryScrobbles,
  scrobbleServiceSummary,
  setLastfmServerCredentials,
  updateScrobbleSettings,
  type ScrobbleServiceStatus,
} from "./scrobbling";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("scrobbling calls the per-profile endpoints", async () => {
  const get = spyOn(api, "get").mockResolvedValue(ok({ services: [] }));
  await getScrobbling();
  expect(get).toHaveBeenCalledWith("/scrobbling");

  const post = spyOn(api, "post").mockResolvedValue(
    ok({ authorizeUrl: "https://www.last.fm/api/auth/?x", expiresIn: 900 }),
  );
  await authorizeLastfm("http://192.168.1.20:1993");
  expect(post).toHaveBeenCalledWith("/scrobbling/lastfm/authorize", {
    origin: "http://192.168.1.20:1993",
  });
  await completeLastfm("state", "token");
  expect(post).toHaveBeenLastCalledWith(
    "/scrobbling/lastfm/complete",
    { state: "state", token: "token" },
    { skipAuth: true },
  );
  post.mockResolvedValue(ok({ queued: 3 }));
  expect(await retryScrobbles()).toBe(3);
  await importScrobbleHistory("listenbrainz");
  expect(post).toHaveBeenLastCalledWith("/scrobbling/listenbrainz/import");

  const put = spyOn(api, "put").mockResolvedValue(ok({}));
  await connectListenbrainz(" 6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11 ");
  expect(put).toHaveBeenCalledWith("/scrobbling/listenbrainz", {
    token: "6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11",
  });
  await setLastfmServerCredentials(" key ", " secret ");
  expect(put).toHaveBeenLastCalledWith("/scrobbling/lastfm/server", {
    apiKey: "key",
    apiSecret: "secret",
  });

  const patch = spyOn(api, "patch").mockResolvedValue(ok({}));
  await updateScrobbleSettings("lastfm", { scrobbleRadio: false });
  expect(patch).toHaveBeenCalledWith("/scrobbling/lastfm", {
    scrobbleRadio: false,
  });

  const remove = spyOn(api, "delete").mockResolvedValue(ok(null));
  await disconnectScrobbling("listenbrainz");
  expect(remove).toHaveBeenCalledWith("/scrobbling/listenbrainz");
});

test("credential checks", () => {
  expect(
    plausibleListenbrainzToken("6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11"),
  ).toBeTrue();
  expect(plausibleListenbrainzToken("nope")).toBeFalse();
  expect(
    plausibleLastfmCredential("0123456789abcdef0123456789ABCDEF"),
  ).toBeTrue();
  expect(plausibleLastfmCredential("0123")).toBeFalse();
});

const service = (
  overrides: Partial<ScrobbleServiceStatus>,
): ScrobbleServiceStatus => ({
  service: "lastfm",
  name: "Last.fm",
  available: true,
  connection: null,
  queue: { pending: 0, failed: 0, sent: 0, ignored: 0, nextAttemptAt: null },
  import: null,
  ...overrides,
});

const connection = {
  accountName: "ada",
  profileUrl: "https://www.last.fm/user/ada",
  enabled: true,
  sendNowPlaying: true,
  scrobbleRadio: true,
  status: "ok" as const,
  lastError: null,
  lastScrobbledAt: null,
  connectedAt: 1,
};

test("service summaries explain the queue in a sentence", () => {
  expect(scrobbleServiceSummary(service({ available: false })).text).toBe(
    "Needs the server's Last.fm API key",
  );
  expect(scrobbleServiceSummary(service({})).tone).toBe("idle");
  expect(
    scrobbleServiceSummary(
      service({ connection: { ...connection, status: "reauth_required" } }),
    ),
  ).toEqual({ tone: "error", text: "Last.fm needs you to connect again" });
  expect(
    scrobbleServiceSummary(
      service({
        connection: { ...connection, lastError: "Service Offline" },
        queue: {
          pending: 4,
          failed: 0,
          sent: 0,
          ignored: 0,
          nextAttemptAt: 1_000 + 600,
        },
      }),
      1_000,
    ),
  ).toEqual({ tone: "warn", text: "4 waiting · retrying in 10 min" });
  expect(
    scrobbleServiceSummary(
      service({ connection: { ...connection, lastScrobbledAt: 1_000 - 120 } }),
      1_000,
    ).text,
  ).toBe("Last scrobble 2 min ago");
  expect(
    scrobbleServiceSummary(
      service({ connection: { ...connection, enabled: false } }),
    ).text,
  ).toBe("Paused");
});
