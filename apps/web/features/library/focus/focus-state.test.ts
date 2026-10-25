import { expect, test } from "bun:test";
import {
  anchoredScrollLeft,
  audioBadge,
  scrollAnchor,
  catalogSongsToQueue,
  compositionSummary,
  focusedAlbumTracks,
  focusQueryKey,
  recordingQueue,
  shuffled,
  facetQueryKey,
  focusCountLabel,
  smartRefreshAge,
  smartRefreshLabel,
  usesFocusEndpoint,
} from "./focus-state";

test("focused headers read like Roon's '1 of 17 albums'", () => {
  const focused = {
    entity: "albums" as const,
    filters: [{ facet: "liked" as const }],
  };
  expect(focusCountLabel(focused, 1, 17)).toBe("1 of 17 albums");
  expect(focusCountLabel({ entity: "songs", filters: [] }, 1, 1)).toBe(
    "1 song",
  );
  expect(focusCountLabel(focused, undefined, 17)).toBeNull();
  expect(focusCountLabel({ entity: "artists", filters: [] }, 1200, 1200)).toBe(
    "1,200 artists",
  );
});

test("only focused or re-sorted views leave the plain catalog endpoint", () => {
  expect(usesFocusEndpoint({ entity: "albums", filters: [] })).toBeFalse();
  expect(
    usesFocusEndpoint({ entity: "albums", filters: [], sort: "artist" }),
  ).toBeFalse();
  expect(
    usesFocusEndpoint({ entity: "albums", filters: [], sort: "date_added" }),
  ).toBeTrue();
  expect(
    usesFocusEndpoint({ entity: "songs", filters: [], text: "night" }),
  ).toBeTrue();
});

test("audio badges summarise lossless depth and rate or lossy bitrate", () => {
  const base = {
    container: null,
    channels: 2,
    bitrate_kbps: null,
    sample_rate: null,
    bit_depth: null,
    lossless: null,
  };
  expect(
    audioBadge({
      ...base,
      codec: "flac",
      sample_rate: 96_000,
      bit_depth: 24,
      lossless: true,
    }),
  ).toBe("FLAC 24/96");
  expect(
    audioBadge({
      ...base,
      codec: "flac",
      sample_rate: 44_100,
      bit_depth: 16,
      lossless: true,
    }),
  ).toBe("FLAC 16/44.1");
  expect(
    audioBadge({
      ...base,
      codec: "pcm",
      container: "wav",
      sample_rate: 192_000,
      bit_depth: 24,
      lossless: true,
    }),
  ).toBe("WAV 24/192");
  expect(
    audioBadge({ ...base, codec: "mp3", bitrate_kbps: 320, lossless: false }),
  ).toBe("MP3 320");
  expect(audioBadge(null)).toBeNull();
});

test("facet keys ignore sort so reordering does not refetch counts", () => {
  const query = {
    entity: "albums" as const,
    filters: [],
    sort: "random" as const,
    seed: 4,
  };
  expect(facetQueryKey(query)).toEqual(
    facetQueryKey({ ...query, sort: "title", seed: 9 }),
  );
});

test("focused songs become queue entries and shuffle keeps every song", () => {
  const songs = ["a", "b", "c"].map((id) => ({
    id,
    name: `Song ${id}`,
    artistId: "artist",
    artistName: "Artist",
    albumId: "album",
    albumName: "Album",
    coverPath: "cover.jpg",
    path: `/${id}.flac`,
    durationSeconds: 60,
  }));
  const queue = catalogSongsToQueue(songs);
  expect(queue[0]).toEqual({
    song: {
      id: "a",
      name: "Song a",
      artist: "Artist",
      path: "/a.flac",
      duration: 60,
    },
    artist: { id: "artist", name: "Artist" },
    album: { id: "album", name: "Album", cover_url: "cover.jpg" },
  });
  const order = shuffled(queue, () => 0).map((item) => item.song.id);
  expect(order.sort()).toEqual(["a", "b", "c"]);
  expect(shuffled([1, 2, 3], () => 0)).toEqual([2, 3, 1]);
});

test("smart playlist refresh times read server UTC timestamps", () => {
  const now = Date.parse("2026-10-24T12:00:00Z");
  expect(smartRefreshAge("2026-10-24 11:59:00", now)).toBe(60_000);
  expect(smartRefreshLabel("2026-10-24 11:59:30", now)).toBe(
    "Updated just now",
  );
  expect(smartRefreshLabel("2026-10-24 11:15:00", now)).toBe(
    "Updated 45 min ago",
  );
  expect(smartRefreshLabel("2026-10-22 12:00:00", now)).toBe(
    "Updated 2 days ago",
  );
  expect(smartRefreshLabel("garbage", now)).toBe("Not refreshed yet");
});

test("album focus keeps track order unless a sort is chosen", () => {
  const tracks = [{ id: "a" }, { id: "b" }, { id: "c" }];
  expect(focusedAlbumTracks(tracks, undefined, false)).toEqual(tracks);
  expect(focusedAlbumTracks(tracks, ["c", "a"], false)).toEqual([
    { id: "a" },
    { id: "c" },
  ]);
  expect(focusedAlbumTracks(tracks, ["c", "missing", "a"], true)).toEqual([
    { id: "c" },
    { id: "a" },
  ]);
});

test("compositions summarise recordings and movements", () => {
  expect(compositionSummary({ movementCount: 1, recordings: [{}] })).toBe(
    "1 recording",
  );
  expect(compositionSummary({ movementCount: 4, recordings: [{}, {}] })).toBe(
    "2 recordings · 4 movements",
  );
});

test("a recording plays its movements with album context", () => {
  const queue = recordingQueue({
    albumId: "al",
    albumName: "Symphonies",
    artistId: "ar",
    artistName: "Berliner Philharmoniker",
    coverPath: null,
    releaseYear: "1978",
    performers: [],
    durationSeconds: 30,
    tracks: [
      {
        id: "t1",
        title: "I. Allegro",
        movement: "Allegro",
        movementNumber: 1,
        durationSeconds: 10,
      },
      {
        id: "t2",
        title: "II. Adagio",
        movement: null,
        movementNumber: null,
        durationSeconds: 20,
      },
    ],
  });
  expect(queue.map((item) => item.song.id)).toEqual(["t1", "t2"]);
  expect(queue[0]?.album).toEqual({
    id: "al",
    name: "Symphonies",
    cover_url: "",
  });
  expect(queue[1]?.song.artist).toBe("Berliner Philharmoniker");
});

test("scoped queries key on their scope", () => {
  const base = { entity: "songs" as const, filters: [] };
  expect(
    facetQueryKey({
      ...base,
      scope: [{ facet: "album", album_id: "a" }],
    }),
  ).not.toEqual(facetQueryKey(base));
  expect(focusQueryKey(base).scope).toEqual([]);
});

test("the focus panel keeps the clicked column in place when columns change", () => {
  const before = [
    { key: "form:", left: 0, width: 180 },
    { key: "instrumentation:", left: 204, width: 180 },
    { key: "period:", left: 408, width: 180 },
    { key: "credit:performer", left: 612, width: 180 },
  ];
  // Scrolled so Period is the first visible column, 20px in.
  const anchor = scrollAnchor(before, 388);
  expect(anchor).toEqual({ key: "period:", offset: 20 });
  // A click removes the Instrumentation column: Period moves left by one
  // column, so the scroll position follows it instead of jumping.
  const after = [
    { key: "form:", left: 0, width: 180 },
    { key: "period:", left: 204, width: 180 },
    { key: "credit:performer", left: 408, width: 180 },
  ];
  expect(anchoredScrollLeft(after, anchor!)).toBe(184);
  // Columns that grow (fewer columns share the width) still anchor.
  const wider = [
    { key: "period:", left: 0, width: 300 },
    { key: "credit:performer", left: 324, width: 300 },
  ];
  expect(
    anchoredScrollLeft(wider, { key: "credit:performer", offset: 0 }),
  ).toBe(324);
  expect(anchoredScrollLeft(after, { key: "gone:", offset: 0 })).toBeNull();
  expect(scrollAnchor([], 0)).toBeNull();
});
