import { expect, test } from "bun:test";
import type { LiveStation } from "@parson/music-sdk";

import { shouldReconnect, stationLogoUrl, stationSong } from "./live-radio";

const station = {
  id: "d1a54d2e-623e-4970-ab11-35f7b56c5ec3",
  song_id: "radio:d1a54d2e-623e-4970-ab11-35f7b56c5ec3",
  source: "directory",
  name: "Radio Paradise",
  stream_url: "https://stream.example/flac",
  homepage: null,
  logo_url:
    "/api/v1/live-radio/stations/d1a54d2e-623e-4970-ab11-35f7b56c5ec3/logo",
  country: "United States",
  country_code: "US",
  region: "California",
  languages: ["english"],
  tags: ["eclectic"],
  codec: "flac",
  bitrate_kbps: null,
  hls: false,
  quality: { tier: "lossless", label: "FLAC", lossless: true },
  votes: null,
  clicks: null,
  click_trend: null,
  online: true,
  description: null,
  allow_lan: false,
  saved: true,
} satisfies LiveStation;

test("stations become live songs with the server's logo", () => {
  const song = stationSong(station, "http://192.168.1.5:1993/");
  expect(song.id).toBe(station.song_id);
  expect(song.duration).toBe(0);
  expect(song.name).toBe("Radio Paradise");
  expect(song.artist).toBe("California, United States · Eclectic");
  expect(song.album_object.cover_url).toBe(
    `http://192.168.1.5:1993${station.logo_url}`,
  );
  expect(stationLogoUrl({ logo_url: null }, "http://x")).toBeNull();
  expect(stationLogoUrl(station, null)).toBeNull();
});

test("dropped streams reconnect a few times a minute at most", () => {
  let attempts: number[] = [];
  for (let index = 0; index < 5; index += 1) {
    const next = shouldReconnect(attempts, 1_000 * index);
    expect(next.reconnect).toBeTrue();
    attempts = next.attempts;
  }
  expect(shouldReconnect(attempts, 6_000).reconnect).toBeFalse();
  expect(shouldReconnect(attempts, 70_000).reconnect).toBeTrue();
});
