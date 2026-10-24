import { expect, test } from "bun:test";
import type {
  StreamingProviderStatus,
  StreamingSearchResponse,
  StreamingTrack,
} from "@parson/music-sdk";

import {
  codeSecondsLeft,
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
