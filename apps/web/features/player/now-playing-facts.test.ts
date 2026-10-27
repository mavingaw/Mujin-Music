import { expect, test } from "bun:test";
import { nowPlayingFacts } from "./now-playing-facts";

test("now playing facts collect album, year, composers and label", () => {
  const facts = nowPlayingFacts({
    track_number: 2,
    label: " Musopen ",
    credits: [
      {
        artist_id: "a",
        name: "J. S. Bach",
        role: "composer",
        instrument: null,
      },
      {
        artist_id: "b",
        name: "Kimiko Ishizaka",
        role: "performer",
        instrument: "piano",
      },
      {
        artist_id: "a",
        name: "J. S. Bach",
        role: "composer",
        instrument: null,
      },
    ],
    album_object: {
      id: "goldberg",
      name: "The Open Goldberg Variations",
      first_release_date: "2012-05-28",
      songs: [{}, {}, {}],
    },
  });
  expect(facts).toEqual({
    albumId: "goldberg",
    albumName: "The Open Goldberg Variations",
    year: "2012",
    composers: ["J. S. Bach"],
    label: "Musopen",
    trackNumber: 2,
    trackCount: 3,
  });
});

test("missing album data falls back to the player's album", () => {
  const facts = nowPlayingFacts(
    { album_object: { first_release_date: "0000" } },
    { id: "x", name: "Fallback", first_release_date: "1994-01-01" },
  );
  expect(facts.albumId).toBe("x");
  expect(facts.albumName).toBe("Fallback");
  expect(facts.year).toBeUndefined();
  expect(facts.composers).toEqual([]);
  expect(facts.label).toBeNull();
  expect(facts.trackCount).toBeUndefined();
});
