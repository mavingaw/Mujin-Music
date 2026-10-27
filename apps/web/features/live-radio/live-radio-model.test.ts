import { expect, test } from "bun:test";
import type { LiveCategory, LiveStation } from "@parson/music-sdk";
import {
  browserRegion,
  categorySearch,
  compactCount,
  hasFilters,
  liveLogoSrc,
  moveId,
  paramsFromSearch,
  searchFromParams,
  searchTitle,
  stationMonogram,
  stationQueueItem,
} from "./live-radio-model";

const station: LiveStation = {
  id: "d1a54d2e-623e-4970-ab11-35f7b56c5ec3",
  song_id: "radio:d1a54d2e-623e-4970-ab11-35f7b56c5ec3",
  source: "directory",
  name: "Classic Vinyl HD",
  stream_url: "https://icecast.example/classic",
  homepage: null,
  logo_url:
    "/api/v1/live-radio/stations/d1a54d2e-623e-4970-ab11-35f7b56c5ec3/logo",
  country: "The United States Of America",
  country_code: "US",
  region: "New York NY",
  languages: ["english"],
  tags: ["jazz", "swing"],
  codec: "mp3",
  bitrate_kbps: 320,
  hls: false,
  quality: { tier: "high", label: "MP3 320", lossless: false },
  votes: 309171,
  clicks: 811,
  click_trend: 12,
  online: true,
  description: null,
  allow_lan: false,
  saved: false,
};

test("stations queue as live tracks with a relayed logo", () => {
  const item = stationQueueItem(station, "http://music.local:1993/");
  expect(item.song.id).toBe(station.song_id);
  expect(item.song.duration).toBe(0);
  expect(item.album?.cover_url).toBe(
    `http://music.local:1993${station.logo_url}`,
  );
  expect(item.artist?.name).toBe("New York NY, United States · Jazz / Swing");
  expect(liveLogoSrc(null, "http://x")).toBeNull();
  expect(liveLogoSrc("data:image/png;base64,AA", "http://x")).toBe(
    "data:image/png;base64,AA",
  );
  expect(liveLogoSrc("https://cdn.example/a.png", "http://x")).toBe(
    "https://cdn.example/a.png",
  );
});

test("searches round-trip through the query string", () => {
  const search = searchFromParams(
    new URLSearchParams(
      "q=jazz&country=fr&lossless=1&min_bitrate=256&order=votes&tag=",
    ),
  );
  expect(search).toEqual({
    name: "jazz",
    tag: undefined,
    country_code: "FR",
    language: undefined,
    codec: undefined,
    lossless: true,
    min_bitrate: 256,
    order: "votes",
  });
  expect(paramsFromSearch(search)).toBe(
    "q=jazz&country=FR&lossless=1&min_bitrate=256&order=votes",
  );
  expect(searchFromParams(new URLSearchParams("order=bogus")).order).toBe(
    undefined,
  );
  expect(hasFilters({})).toBeFalse();
  expect(hasFilters({ order: "trending" })).toBeTrue();
  expect(hasFilters({ lossless: true })).toBeTrue();
});

test("titles name the category, country and language", () => {
  const categories: LiveCategory[] = [
    {
      id: "lossless",
      label: "Lossless",
      group: "collections",
      lossless: true,
      colors: ["#000", "#111"],
    },
    {
      id: "jazz",
      label: "Jazz",
      group: "music",
      tag: "jazz",
      colors: ["#000", "#111"],
    },
  ];
  const names = (code: string) => (code === "FR" ? "France" : null);
  expect(
    searchTitle({ tag: "jazz", country_code: "FR" }, categories, names),
  ).toBe("Jazz · France");
  expect(searchTitle(categorySearch(categories[0]!), categories, names)).toBe(
    "Lossless",
  );
  expect(
    searchTitle({ name: "fip", language: "french" }, categories, names),
  ).toBe("“fip” · French");
  expect(searchTitle({}, categories, names)).toBe("Stations");
});

test("monograms are stable and readable", () => {
  expect(stationMonogram("Radio Paradise").initials).toBe("RP");
  expect(stationMonogram("KEXP").initials).toBe("KE");
  expect(stationMonogram("!!!").initials).toBe("FM");
  expect(stationMonogram("Radio Paradise")).toEqual(
    stationMonogram("Radio Paradise"),
  );
});

test("helpers for regions, counts and reordering", () => {
  expect(browserRegion(["en-GB", "en"])).toBe("GB");
  expect(browserRegion(["fr"])).toBeNull();
  expect(compactCount(811)).toBe("811");
  expect(compactCount(12_400)).toBe("12.4k");
  expect(compactCount(309_171)).toBe("309k");
  expect(compactCount(2_000_000)).toBe("2M");
  expect(compactCount(0)).toBeNull();
  expect(moveId(["a", "b", "c"], 0, 2)).toEqual(["b", "c", "a"]);
  expect(moveId(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
  expect(moveId(["a"], 3, 0)).toEqual(["a"]);
});
