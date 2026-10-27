import { expect, test } from "bun:test";
import type { CatalogFacets } from "@parson/music-sdk";
import { focusChips, focusFilter, noFocus } from "./catalog-focus";

const facets: CatalogFacets = {
  sources: [
    { value: "local", count: 12 },
    { value: "tidal", count: 3 },
    { value: "qobuz", count: 0 },
  ],
  genres: [
    { value: "Jazz", count: 6 },
    { value: "Pop/Rock", count: 5 },
    { value: "Classical", count: 1 },
  ],
  decades: [
    { value: 1990, count: 4 },
    { value: 2020, count: 11 },
  ],
  qualities: [],
};

test("focus filters only carry what is set", () => {
  expect(focusFilter(noFocus)).toEqual({});
  expect(focusFilter({ source: "tidal", genre: "Jazz", decade: 1990 })).toEqual(
    { source: "tidal", genre: "Jazz", decade: 1990 },
  );
});

test("chips combine source, genre and decade and toggle off", () => {
  const chips = focusChips(facets, noFocus, 2);
  expect(chips.map((chip) => chip.label)).toEqual([
    "Local files",
    "TIDAL library",
    "Jazz",
    "Pop/Rock",
    "1990s",
    "2020s",
  ]);
  const tidal = chips.find((chip) => chip.key === "source:tidal");
  expect(tidal?.next).toEqual({ source: "tidal", genre: null, decade: null });

  const focused = focusChips(
    facets,
    { source: "tidal", genre: "Classical", decade: null },
    2,
  );
  expect(focused.slice(0, 2).map((chip) => chip.label)).toEqual([
    "TIDAL library",
    "Classical",
  ]);
  expect(focused[1]?.next.genre).toBeNull();
  expect(focusChips(undefined, noFocus)).toEqual([]);
});

test("a single source needs no source chips", () => {
  const localOnly = {
    ...facets,
    sources: [{ value: "local" as const, count: 12 }],
  };
  expect(
    focusChips(localOnly, noFocus).some((chip) =>
      chip.key.startsWith("source:"),
    ),
  ).toBe(false);
});
