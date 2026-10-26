import { expect, test } from "bun:test";
import {
  chipTone,
  facetChipAccessibilityLabel,
  focusEntityForSection,
  recordingCaption,
  recordingSongs,
  smartPlaylistName,
  durationLabel,
  workCaption,
} from "./library-focus";

test("only catalog browsers have a Focus", () => {
  expect(focusEntityForSection("albums")).toBe("albums");
  expect(focusEntityForSection("artists")).toBe("artists");
  expect(focusEntityForSection("composers")).toBe("composers");
  expect(focusEntityForSection("downloads")).toBeNull();
  expect(focusEntityForSection("liked")).toBeNull();
});

test("smart playlists are named after their focus", () => {
  expect(smartPlaylistName({ entity: "albums", filters: [] })).toBe(
    "Smart playlist",
  );
  expect(
    smartPlaylistName({
      entity: "albums",
      filters: [
        { facet: "quality", value: "hires" },
        { facet: "played", exclude: true },
      ],
    }),
  ).toBe("Hi-Res · Unplayed");
  const long = smartPlaylistName({
    entity: "songs",
    filters: [{ facet: "genre", value: "x".repeat(300) }],
  });
  expect(long.length).toBe(200);
});

test("facet chips describe their state for screen readers", () => {
  expect(chipTone(null)).toBe("idle");
  expect(facetChipAccessibilityLabel("Jazz", 2, null)).toBe(
    "Jazz, 2 items. Double tap to focus.",
  );
  expect(facetChipAccessibilityLabel("MP3", 1, "exclude")).toBe(
    "MP3, excluded. Double tap to clear.",
  );
});

test("a composition recording plays in movement order with album context", () => {
  const recording = {
    albumId: "al",
    albumName: "Symphonies",
    artistId: "ar",
    artistName: "Berliner Philharmoniker",
    coverPath: "cover.jpg",
    releaseYear: "1978",
    performers: ["Claudio Abbado", "Berliner Philharmoniker"],
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
        movement: "Adagio",
        movementNumber: 2,
        durationSeconds: 20,
      },
    ],
  };
  const songs = recordingSongs(recording);
  expect(songs.map((song) => song.id)).toEqual(["t1", "t2"]);
  expect(songs[0]?.album_object.cover_url).toBe("cover.jpg");
  expect(songs[1]?.duration).toBe(20);
  expect(recordingCaption(recording)).toBe(
    "Claudio Abbado, Berliner Philharmoniker · 1978",
  );
  expect(
    recordingCaption({ ...recording, performers: [], releaseYear: null }),
  ).toBe("Berliner Philharmoniker");
});

test("compositions show their composer, form, scoring and date", () => {
  expect(
    workCaption({
      composers: [{ id: "b", name: "Ludwig van Beethoven" }],
      form: "Symphony",
      instrumentation: "Orchestra",
      composedYear: 1812,
    }),
  ).toBe("Ludwig van Beethoven · Symphony for Orchestra · Composed 1812");
  expect(workCaption({ composers: [], form: null })).toBe("");
  expect(focusEntityForSection("compositions")).toBe("compositions");
  expect(durationLabel(245)).toBe("4:05");
  expect(durationLabel(0)).toBe("");
});
