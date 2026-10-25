import { expect, test } from "bun:test";
import type {
  StreamingDelivery,
  StreamingProviderStatus,
  StreamingSearchResponse,
  StreamingTrack,
} from "@parson/music-sdk";

import {
  codeSecondsLeft,
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
  expect(deliveryText({ ...delivery, delivered: null }).label).toBe(
    "Format unknown",
  );
});
