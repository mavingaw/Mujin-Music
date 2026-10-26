import { expect, test } from "bun:test";
import type {
  StreamingDelivery,
  StreamingProviderStatus,
  StreamingSearchResponse,
  StreamingTrack,
} from "@parson/music-sdk";

import {
  codeSecondsLeft,
  editorialPreview,
  keysNoticeText,
  signInThroughText,
  subscriberSetupText,
  tidalPhoneSignIn,
  deliveryText,
  needsOwnCredentials,
  serviceAction,
  streamingSearchRows,
} from "./streaming";

const track = (id: string, streamable = true) =>
  ({
    id,
    provider: "qobuz",
    title: id,
    streamable,
  }) as unknown as StreamingTrack;

test("search rows group each service under its own header", () => {
  const response = {
    query: "harbor",
    services: [
      {
        provider: "qobuz",
        name: "Qobuz",
        results: {
          albums: [{ id: "qobuz:a" }],
          tracks: [track("qobuz:1"), track("qobuz:2", false)],
          artists: [],
        },
      },
      { provider: "tidal", name: "TIDAL", error: "Your session expired." },
    ],
  } as unknown as StreamingSearchResponse;
  const rows = streamingSearchRows(response);
  expect(rows.map((row) => row.type)).toEqual([
    "service",
    "album",
    "track",
    "service",
  ]);
  const playable = rows.find((row) => row.type === "track");
  expect(playable?.type === "track" && playable.queue.length).toBe(1);
  expect(rows.at(-1)).toMatchObject({ error: "Your session expired." });
  expect(streamingSearchRows(undefined)).toEqual([]);
});

test("service actions and code countdowns", () => {
  const base = {
    status: "configured",
    credentials: { configured: false },
  } as unknown as StreamingProviderStatus;
  expect(serviceAction(base)).toBe("Set up");
  expect(
    serviceAction({
      ...base,
      credentials: { ...base.credentials, configured: true },
    }),
  ).toBe("Sign in");
  expect(serviceAction({ ...base, status: "signed_in" })).toBe("Manage");
  expect(codeSecondsLeft(100, 40_000)).toBe(60);
  expect(codeSecondsLeft(100, 200_000)).toBe(0);
});

test("server credentials make sign-in the first step", () => {
  const provider = {
    status: "configured",
    credentials: { configured: false },
    server_credentials_available: true,
  } as unknown as StreamingProviderStatus;
  expect(needsOwnCredentials(provider)).toBeFalse();
  expect(serviceAction(provider)).toBe("Sign in");
  expect(
    needsOwnCredentials({ ...provider, server_credentials_available: false }),
  ).toBeTrue();
});

test("the player line names the delivered format", () => {
  const flac = (label: string, depth: number, rate: number) => ({
    codec: "flac",
    lossless: true,
    bit_depth: depth,
    sample_rate: rate,
    bitrate_kbps: null,
    channels: 2,
    label,
  });
  const delivery: StreamingDelivery = {
    track_id: "qobuz:7700001",
    requested: "hi_res_max",
    announced: flac("24/192 FLAC", 24, 192000),
    delivered: flac("24/192 FLAC", 24, 192000),
    container: "flac",
    frame_verified: true,
    matches_announced: true,
    sandbox: false,
    at: 0,
  };
  expect(deliveryText(delivery)).toEqual({
    label: "24/192 FLAC",
    detail: "delivered · stereo · frame verified",
    warning: null,
  });
  expect(
    deliveryText({
      ...delivery,
      delivered: flac("16/44.1 FLAC", 16, 44100),
      matches_announced: false,
    }).warning,
  ).toBe("Announced 24/192 FLAC");
  expect(deliveryText({ ...delivery, sandbox: true, tone: false })).toEqual({
    label: "24/192 FLAC",
    detail: "sandbox, synthesized audio · stereo · frame verified",
    warning: null,
  });
  expect(
    deliveryText({ ...delivery, sandbox: true, matches_announced: false })
      .detail,
  ).toBe("sandbox preview · stereo · frame verified");
  expect(deliveryText({ ...delivery, delivered: null }).label).toBe(
    "Format unknown",
  );

  // A short lossy bitrate names what disagrees.
  const aac = (kbps: number) => ({
    codec: "aac",
    lossless: false,
    bit_depth: null,
    sample_rate: 44100,
    bitrate_kbps: kbps,
    channels: 2,
    label: `AAC ${kbps}`,
  });
  expect(
    deliveryText({
      ...delivery,
      announced: aac(320),
      delivered: aac(255),
      matches_announced: false,
      mismatches: ["255 kbps, not 320"],
    }).warning,
  ).toBe("Announced AAC 320 (255 kbps, not 320)");

  // Full length or a preview, against the server's or the player's length.
  expect(
    deliveryText({ ...delivery, delivered_seconds: 241, full_length: true })
      .detail,
  ).toBe("delivered · stereo · frame verified · full length 4:01");
  expect(
    deliveryText({ ...delivery, delivered_seconds: 30 }, 241).warning,
  ).toBe("Preview: 0:30 of 4:01");
  expect(
    deliveryText(
      { ...delivery, sandbox: true, tone: false, delivered_seconds: 30 },
      241,
    ).warning,
  ).toBeNull();
});

test("reviews show their first paragraph until expanded", () => {
  const text = "First paragraph.\n\nSecond paragraph.";
  expect(editorialPreview(text, false)).toEqual({
    paragraphs: ["First paragraph."],
    more: true,
  });
  expect(editorialPreview(text, true).paragraphs).toHaveLength(2);
  expect(editorialPreview("Short.", false)).toEqual({
    paragraphs: ["Short."],
    more: false,
  });
  const long = editorialPreview("x".repeat(900), false);
  expect(long.more).toBe(true);
  expect(long.paragraphs[0]?.length).toBe(398);
});

test("subscribers are told to ask the server owner, not to find API keys", () => {
  const tidal = subscriberSetupText("tidal", "TIDAL");
  expect(tidal).toContain("approve the sign-in on TIDAL");
  expect(tidal).toContain("Server credentials");
  expect(subscriberSetupText("qobuz", "Qobuz")).toContain(
    "Qobuz email and password",
  );
});

test("a key service means a subscription is all a listener needs", () => {
  const provider = {
    id: "qobuz",
    name: "Qobuz",
    status: "configured",
    credentials: {
      configured: false,
      client_id_hint: null,
      has_secret: false,
      uses_server_credentials: false,
    },
    server_credentials_available: true,
    server_credentials_source: "key_service",
    key_service: {
      url: "https://keys.parson.test",
      issuer: "Parson",
      offers: true,
      device_code: false,
      error: null,
    },
  } as unknown as StreamingProviderStatus;
  expect(needsOwnCredentials(provider)).toBeFalse();
  expect(signInThroughText(provider)).toBe(
    "Signs in through Parson's licensed Qobuz app · Use your own",
  );
  expect(keysNoticeText([provider])).toContain(
    "Sign in to Qobuz with just your subscription",
  );
  const tidal = { ...provider, id: "tidal", name: "TIDAL" };
  expect(tidalPhoneSignIn(tidal).canStart).toBeFalse();
  expect(
    tidalPhoneSignIn({
      ...tidal,
      key_service: { ...provider.key_service!, device_code: true },
    }).canStart,
  ).toBeTrue();
  const none = {
    ...provider,
    server_credentials_available: false,
    server_credentials_source: null,
    key_service: null,
  };
  expect(keysNoticeText([none])).toContain("no key service");
  expect(tidalPhoneSignIn({ ...none, id: "tidal" }).canStart).toBeTrue();
});
