import { expect, test } from "bun:test";
import { completeNowPlayingSong, nowPlayingFacts } from "./now-playing-facts";

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

test("after Next or auto-advance a lean queue entry keeps the full facts table", () => {
  // What a queue entry reached by Next carries: no credits, no label, no
  // track list, a placeholder date.
  const lean = {
    track_number: 3,
    album_object: {
      id: "goldberg",
      name: "The Open Goldberg Variations",
      first_release_date: "0000",
    },
  };
  // The same song and album as the server knows them.
  const fetchedSong = {
    track_number: 3,
    label: null,
    credits: [
      {
        artist_id: "bach",
        name: "Johann Sebastian Bach",
        role: "composer",
        instrument: null,
      },
    ],
    album_object: {
      id: "goldberg",
      name: "The Open Goldberg Variations",
      first_release_date: "2012",
      songs: [],
    },
  };
  const fetchedAlbum = {
    id: "goldberg",
    name: "The Open Goldberg Variations",
    first_release_date: "2012-05-28",
    label: "Open Goldberg",
    songs: [{}, {}, {}, {}, {}],
  };
  expect(
    nowPlayingFacts(completeNowPlayingSong(lean, fetchedSong, fetchedAlbum)),
  ).toEqual({
    albumId: "goldberg",
    albumName: "The Open Goldberg Variations",
    year: "2012",
    composers: ["Johann Sebastian Bach"],
    label: "Open Goldberg",
    trackNumber: 3,
    trackCount: 5,
  });
  // Before the server answers, the table shows what the entry has.
  expect(nowPlayingFacts(completeNowPlayingSong(lean)).albumName).toBe(
    "The Open Goldberg Variations",
  );
});

test("another album's details never leak into the facts", () => {
  const playing = {
    track_number: 1,
    album_object: { id: "a", name: "A", first_release_date: "1999" },
  };
  const facts = nowPlayingFacts(
    completeNowPlayingSong(playing, null, {
      id: "b",
      name: "B",
      first_release_date: "2010",
      label: "Other",
      songs: [{}, {}],
    }),
  );
  expect(facts.year).toBe("1999");
  expect(facts.label).toBeNull();
  expect(facts.trackCount).toBeUndefined();
});
