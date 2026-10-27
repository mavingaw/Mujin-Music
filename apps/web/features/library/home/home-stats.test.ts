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
    { label: "Library length", value: "43", unit: "min" },
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
    label: "Library length",
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

test("time listened comes from history, not from the library's length", () => {
  const profile = {
    composer_count: 6,
    genre_count: 4,
    total_duration_seconds: 1080,
    lossless_tracks: 18,
    hi_res_tracks: 9,
    hi_res_albums: 3,
  };
  const before = homeStats(
    feed({ profile, listening: { plays: 0, listened_seconds: 0 } }),
  );
  expect(before.find((stat) => stat.label === "Library length")).toEqual({
    label: "Library length",
    value: "18",
    unit: "min",
  });
  // Nothing played yet reads as zero, not as the library's 18 minutes.
  expect(before.find((stat) => stat.label === "You've listened")).toEqual({
    label: "You've listened",
    value: "0",
    unit: "min",
  });
  const after = homeStats(
    feed({ profile, listening: { plays: 40, listened_seconds: 9_000 } }),
  );
  expect(after.find((stat) => stat.label === "You've listened")?.value).toBe(
    "3",
  );
  expect(after.find((stat) => stat.label === "You've listened")?.unit).toBe(
    "h",
  );
  // Older servers send no history: no listening card at all.
  expect(
    homeStats(feed({ profile })).some(
      (stat) => stat.label === "You've listened",
    ),
  ).toBe(false);
});
