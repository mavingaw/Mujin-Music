import { expect, test } from "bun:test";
import type { GenreRef, GenreTree } from "../domain/types";
import {
  flattenGenreTree,
  genreAboutWaitingLabel,
  genreChipTitle,
  genreDisplayName,
  genreHref,
  genreOriginCaption,
  genrePathLabel,
  genreSuggestions,
} from "./genres";

const hardBop: GenreRef = {
  slug: "hard-bop",
  name: "Hard Bop",
  path: ["Jazz"],
  canonical: true,
  sources: ["musicbrainz", "tags"],
  raw_names: ["hard bop"],
};

const tree: GenreTree = {
  source: "online",
  genres: [
    {
      slug: "jazz",
      name: "Jazz",
      album_count: 3,
      children: [
        { slug: "hard-bop", name: "Hard Bop", album_count: 2, children: [] },
      ],
    },
  ],
  other: [{ slug: "krautrock", name: "Krautrock", album_count: 1 }],
  catalog: [
    { slug: "jazz", name: "Jazz", parent: null },
    { slug: "hard-bop", name: "Hard Bop", parent: "jazz" },
    { slug: "bebop", name: "Bebop", parent: "jazz" },
    { slug: "jazz-fusion", name: "Jazz Fusion", parent: "jazz" },
    { slug: "acid-jazz", name: "Acid Jazz", parent: "jazz" },
  ],
};

test("genre chips link to the page and name their path and origins", () => {
  expect(genreHref("hard-bop")).toBe("/genre?slug=hard-bop");
  expect(genreHref("neue-deutsche-härte")).toBe(
    "/genre?slug=neue-deutsche-h%C3%A4rte",
  );
  expect(genrePathLabel(hardBop)).toBe("Jazz › Hard Bop");
  expect(genreChipTitle(hardBop)).toBe(
    "Jazz › Hard Bop · from MusicBrainz and file tags",
  );
  expect(genreOriginCaption([hardBop])).toBe("MusicBrainz and file tags");
  expect(genreOriginCaption([{ ...hardBop, sources: ["user"] }])).toBe(
    "Edited",
  );
  expect(genreOriginCaption([])).toBeNull();
});

test("the tree flattens with depth and suggestions rank word starts first", () => {
  expect(flattenGenreTree(tree.genres).map((genre) => genre.depth)).toEqual([
    0, 1,
  ]);
  expect(genreSuggestions(tree, "jazz", [])).toEqual([
    "Jazz",
    "Jazz Fusion",
    "Acid Jazz",
  ]);
  expect(genreSuggestions(tree, "bop", ["Hard Bop"])).toEqual(["Bebop"]);
  expect(genreSuggestions(tree, "kraut", [])).toEqual(["Krautrock"]);
  expect(genreSuggestions(null, "x", [])).toEqual([]);
});

test("genre pages say what the description is waiting for", () => {
  expect(genreAboutWaitingLabel(null)).toBeNull();
  expect(
    genreAboutWaitingLabel({
      enabled: true,
      task: {
        status: "pending",
        last_error: null,
        updated_at: "",
        stage: "wikidata",
      },
    }),
  ).toBe("Finding this genre on Wikidata.");
  expect(
    genreAboutWaitingLabel({
      enabled: true,
      task: {
        status: "pending",
        last_error: null,
        updated_at: "",
        stage: "wikipedia",
      },
    }),
  ).toBe("Fetching the description from Wikipedia.");
  expect(genreAboutWaitingLabel({ enabled: false, task: null })).toBeNull();
});

test("MusicBrainz's lower-case genres read as titles; capitals stay", () => {
  expect(genreDisplayName("industrial rock")).toBe("Industrial Rock");
  expect(genreDisplayName("electro-industrial")).toBe("Electro-Industrial");
  expect(genreDisplayName("drum and bass")).toBe("Drum and Bass");
  expect(genreDisplayName("the blues")).toBe("The Blues");
  expect(genreDisplayName("IDM")).toBe("IDM");
  expect(genreDisplayName("Hip Hop")).toBe("Hip Hop");
  expect(genreDisplayName("J-Pop")).toBe("J-Pop");
  expect(genreDisplayName("électronique")).toBe("Électronique");
});
