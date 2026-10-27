import { expect, test } from "bun:test";
import type { HomeEssentials } from "@parson/music-sdk";
import { homeStats } from "./home-stats";

const feed = (overrides: Partial<HomeEssentials> = {}): HomeEssentials => ({
  continue_listening: [],
  recommended: [],
  shuffle: [],
  albums: [],
  stats: { song_count: 54, album_count: 15, artist_count: 14 },
  ...overrides,
});

test("an empty library shows no stat cards", () => {
  expect(
    homeStats(
      feed({ stats: { song_count: 0, album_count: 0, artist_count: 0 } }),
    ),
  ).toEqual([]);
});

test("older servers get the three catalogue counts", () => {
  expect(homeStats(feed()).map((stat) => stat.label)).toEqual([
    "Artists",
    "Albums",
    "Tracks",
  ]);
});

test("the library profile adds composers, hours and the lossless share", () => {
  const stats = homeStats(
    feed({
      profile: {
        composer_count: 1,
        genre_count: 12,
        total_duration_seconds: 2592,
        lossless_tracks: 50,
        hi_res_tracks: 5,
        hi_res_albums: 1,
      },
    }),
  );
  expect(stats.slice(3)).toEqual([
    { label: "Composers", value: "1" },
    { label: "Listening time", value: "43", unit: "min" },
    { label: "Lossless · 1 hi-res", value: "93", unit: "%", accent: true },
  ]);
  const long = homeStats(
    feed({
      profile: {
        composer_count: 0,
        genre_count: 0,
        total_duration_seconds: 7_300_000,
        lossless_tracks: 0,
        hi_res_tracks: 0,
        hi_res_albums: 0,
      },
    }),
  );
  expect(long[4]).toEqual({
    label: "Listening time",
    value: "2,028",
    unit: "h",
  });
  expect(long[5]).toEqual({
    label: "Lossless",
    value: "0",
    unit: "%",
    accent: false,
  });
});
