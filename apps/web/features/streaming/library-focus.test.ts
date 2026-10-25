import { describe, expect, test } from "bun:test";
import type { CatalogAlbum } from "@parson/music-sdk";
import {
  activeFocusCount,
  catalogAlbumEntry,
  decadeLabel,
  describeFocus,
  emptyFocus,
  parseLibraryFocus,
  splitFacets,
  toCatalogFilter,
  writeLibraryFocus,
} from "./library-focus";

describe("library focus", () => {
  test("round-trips through the URL and ignores invalid values", () => {
    const focus = parseLibraryFocus(
      new URLSearchParams(
        "view=albums&source=qobuz&genre=Pop%2FRock&decade=1990&quality=hi_res",
      ),
    );
    expect(focus).toEqual({
      source: "qobuz",
      genre: "Pop/Rock",
      decade: 1990,
      quality: "hi_res",
    });
    const params = writeLibraryFocus(new URLSearchParams("view=albums"), focus);
    expect(params.toString()).toBe(
      "view=albums&source=qobuz&genre=Pop%2FRock&decade=1990&quality=hi_res",
    );
    expect(parseLibraryFocus(params)).toEqual(focus);

    expect(
      parseLibraryFocus(
        new URLSearchParams("source=spotify&decade=1995&quality=mqa&genre=+"),
      ),
    ).toEqual(emptyFocus);
    expect(
      writeLibraryFocus(
        new URLSearchParams("source=tidal&x=1"),
        emptyFocus,
      ).toString(),
    ).toBe("x=1");
  });

  test("builds the catalog filter and counts active filters", () => {
    expect(toCatalogFilter(emptyFocus)).toEqual({});
    expect(activeFocusCount(emptyFocus)).toBe(0);
    const focus = {
      source: "tidal" as const,
      genre: "Jazz",
      decade: 2020,
      quality: null,
    };
    expect(toCatalogFilter(focus)).toEqual({
      source: "tidal",
      genre: "Jazz",
      decade: 2020,
    });
    expect(activeFocusCount(focus)).toBe(3);
    expect(decadeLabel(1990)).toBe("1990-1999");
    expect(describeFocus(focus, { tidal: "TIDAL" })).toBe(
      "Jazz, 2020-2029 in TIDAL library",
    );
    expect(describeFocus({ ...emptyFocus, source: "local" }, {})).toBe(
      "in local files",
    );
    expect(describeFocus(emptyFocus, {})).toBeNull();
  });

  test("keeps the selected facet visible", () => {
    const facets = [
      { value: "Rock", count: 9 },
      { value: "Jazz", count: 5 },
      { value: "Folk", count: 1 },
    ];
    expect(splitFacets(facets, null, 2).shown.map((f) => f.value)).toEqual([
      "Rock",
      "Jazz",
    ]);
    const withFolk = splitFacets(facets, "Folk", 2);
    expect(withFolk.shown.map((f) => f.value)).toEqual([
      "Rock",
      "Jazz",
      "Folk",
    ]);
    expect(withFolk.more).toEqual([]);
    expect(splitFacets(facets, "Polka", 5).shown.at(-1)).toEqual({
      value: "Polka",
      count: 0,
    });
  });

  test("catalog albums become library tiles with source badges", () => {
    const album: CatalogAlbum = {
      key: "qobuz:q0harborhr04",
      id: "qobuz:q0harborhr04",
      name: "Harbor Lights",
      artist_id: "qobuz:51004",
      artist_name: "Mara Vell",
      cover: "https://static.qobuz.test/cover.jpg",
      year: 2024,
      genres: ["Pop/Rock"],
      source: "qobuz",
      format_label: "24/192 FLAC",
      quality: "hi_res",
    };
    expect(catalogAlbumEntry(album)).toMatchObject({
      id: "qobuz:q0harborhr04",
      year: "2024",
      source: "qobuz",
      formatLabel: "24/192 FLAC",
    });
    expect(
      catalogAlbumEntry({ ...album, source: "local", year: null }),
    ).toMatchObject({ source: null, year: "" });
  });
});
