import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  createSmartPlaylist,
  getFocusCompositions,
  getFocusFacets,
  getFocusPerson,
  getFocusPage,
  reorderSavedFocuses,
  tagItems,
  type FocusQuery,
} from "./focus";
import {
  albumScope,
  clearedFocus,
  composerScope,
  criterionLabel,
  entityNoun,
  focusCountLabel,
  scopedFocus,
  usesFocusEndpoint,
  cycleFilter,
  decadeCriterion,
  decadesFromYears,
  decodeFocus,
  effectiveDirection,
  effectiveSort,
  emptyFocus,
  encodeFocus,
  facetBaseQuery,
  facetTitle,
  facetValueState,
  filterKey,
  filterState,
  focusSummary,
  toggleFacetValue,
  visibleFacets,
  withEntity,
} from "./focus-model";

afterEach(() => {
  mock.restore();
});

const pop = { facet: "genre", value: "Pop/Rock" } as const;

describe("focus filters", () => {
  test("a facet value cycles include, exclude, then off like Roon", () => {
    let query = emptyFocus("albums");
    query = cycleFilter(query, pop);
    expect(filterState(query, pop)).toBe("include");
    query = cycleFilter(query, pop);
    expect(filterState(query, pop)).toBe("exclude");
    expect(query.filters).toHaveLength(1);
    query = cycleFilter(query, pop);
    expect(filterState(query, pop)).toBeNull();
    expect(query.filters).toHaveLength(0);
  });

  test("genre identity ignores case and range facets hold one range", () => {
    expect(filterKey({ facet: "genre", value: " pop/rock" })).toBe(
      filterKey(pop),
    );
    let query = cycleFilter(emptyFocus(), decadeCriterion(1990));
    query = cycleFilter(query, decadeCriterion(2020));
    expect(query.filters).toEqual([
      { facet: "year", from: 2020, to: 2029, exclude: false },
    ]);
    expect(filterState(query, decadeCriterion(1990))).toBeNull();
  });

  test("chips describe filters and exclusions", () => {
    expect(criterionLabel(decadeCriterion(1970))).toBe("1970s");
    expect(criterionLabel({ facet: "year", from: 2020 })).toBe("≥ 2020");
    expect(criterionLabel({ facet: "sample_rate", min: 88_200 })).toBe(
      "≥ 88.2 kHz",
    );
    expect(criterionLabel({ facet: "format", value: "mp3" })).toBe("MP3");
    expect(
      criterionLabel({
        facet: "credit",
        role: "composer",
        artist_id: "a",
        name: "Abe Rounds",
      }),
    ).toBe("Abe Rounds · Composer");
    expect(criterionLabel({ facet: "played", exclude: true })).toBe("Unplayed");
    const summary = focusSummary({
      entity: "albums",
      filters: [
        { ...pop },
        { ...decadeCriterion(1960) },
        { facet: "release_type", value: "single", exclude: true },
      ],
    });
    expect(summary).toBe("Pop/Rock · 1960s · not Single");
    expect(
      focusSummary({
        entity: "albums",
        match: "any",
        filters: [
          { facet: "genre", value: "Jazz" },
          { facet: "format", value: "mp3", exclude: true },
          { facet: "genre", value: "Pop" },
        ],
      }),
    ).toBe("Jazz or Pop · not MP3");
  });

  test("match any counts facets over the alternatives, not the matches", () => {
    const query: FocusQuery = {
      entity: "albums",
      match: "any",
      filters: [{ ...pop }, { facet: "format", value: "mp3", exclude: true }],
    };
    expect(facetBaseQuery(query).filters).toEqual([
      { facet: "format", value: "mp3", exclude: true },
    ]);
    expect(facetBaseQuery({ ...query, match: "all" })).toEqual({
      ...query,
      match: "all",
    });
  });

  test("played and unplayed are two sides of one status filter", () => {
    const facet = { key: "status" as const };
    const unplayed = { value: "unplayed", label: "Unplayed", count: 3 };
    const played = { value: "played", label: "Played", count: 2 };
    let query = toggleFacetValue(emptyFocus(), facet, unplayed);
    expect(query.filters).toEqual([{ facet: "played", exclude: true }]);
    expect(facetValueState(query, facet, unplayed)).toBe("include");
    expect(facetValueState(query, facet, played)).toBeNull();
    query = toggleFacetValue(query, facet, played);
    expect(query.filters).toEqual([{ facet: "played", exclude: false }]);
    query = toggleFacetValue(query, facet, played);
    expect(query.filters).toEqual([]);
  });

  test("years group into decades and facets get Roon-like titles", () => {
    expect(
      decadesFromYears([
        { value: "1994", label: "1994", count: 2 },
        { value: "1999", label: "1999", count: 1 },
        { value: "1978", label: "1978", count: 4 },
      ]),
    ).toEqual([
      { decade: 1970, count: 4 },
      { decade: 1990, count: 3 },
    ]);
    expect(facetTitle({ key: "credit", role: "producer" })).toBe("Production");
    expect(facetTitle({ key: "credit", role: "composer" })).toBe("Composers");
    expect(
      visibleFacets(
        [
          {
            key: "source",
            total: 1,
            values: [{ value: "1", label: "Music", count: 4 }],
          },
          {
            key: "genre",
            total: 1,
            values: [{ value: "Jazz", label: "Jazz", count: 4 }],
          },
        ],
        "albums",
      ).map((facet) => facet.key),
    ).toEqual(["genre"]);
    const [status] = visibleFacets(
      [
        {
          key: "status",
          total: 3,
          values: [
            { value: "liked", label: "Liked", count: 0 },
            { value: "played", label: "Played", count: 0 },
            { value: "unplayed", label: "Unplayed", count: 3 },
          ],
        },
      ],
      "albums",
    );
    expect(status?.values.map((value) => value.value)).toEqual(["unplayed"]);
    expect(status?.total).toBe(1);
  });

  test("sorts fall back per browser and keep sensible directions", () => {
    const query: FocusQuery = { entity: "albums", filters: [], sort: "album" };
    expect(effectiveSort(query)).toBe("artist");
    expect(effectiveDirection({ ...query, sort: "date_added" })).toBe("desc");
    const artists = withEntity({ ...query, sort: "release_date" }, "artists");
    expect(artists.sort).toBeNull();
    expect(withEntity({ ...query, sort: "most_played" }, "songs").sort).toBe(
      "most_played",
    );
  });

  test("focus survives a URL round trip and rejects malformed input", () => {
    const query: FocusQuery = {
      entity: "songs",
      match: "any",
      filters: [{ ...pop, exclude: true }, { facet: "liked" }],
      text: " night ",
      sort: "random",
      seed: 42,
    };
    const decoded = decodeFocus(encodeFocus(query), "songs");
    expect(decoded.filters).toEqual(query.filters);
    expect(decoded.match).toBe("any");
    expect(decoded.text).toBe("night");
    expect(decoded.seed).toBe(42);
    expect(decodeFocus("{not json", "albums")).toEqual(emptyFocus("albums"));
    expect(
      decodeFocus(
        JSON.stringify({ filters: [{ facet: "drop table" }] }),
        "albums",
      ).filters,
    ).toEqual([]);
  });
});

describe("focus resources", () => {
  test("pages, facets and saved focus order use the library focus routes", async () => {
    const post = spyOn(api, "post").mockResolvedValue({
      data: { total: 0, facets: [] },
    } as never);
    const put = spyOn(api, "put").mockResolvedValue({ data: [] } as never);
    const query: FocusQuery = { entity: "albums", filters: [pop] };

    await getFocusPage(query, 100, 50);
    expect(post).toHaveBeenCalledWith("/library/focus", {
      query,
      offset: 100,
      limit: 50,
    });
    await getFocusFacets(query, { only: ["genre"], search: "  pop " });
    expect(post).toHaveBeenLastCalledWith("/library/focus/facets", {
      query,
      only: ["genre"],
      search: "pop",
      limit: undefined,
    });
    await createSmartPlaylist(" 90s ", query, { description: "Pop/Rock" });
    expect(post).toHaveBeenLastCalledWith("/library/focus/playlists", {
      name: "90s",
      query,
      description: "Pop/Rock",
      limit: undefined,
    });
    await reorderSavedFocuses([3, 1]);
    expect(put).toHaveBeenCalledWith("/library/focus/saved/order", {
      ids: [3, 1],
    });
  });

  test("tagging many items is batched and deduplicated", async () => {
    const post = spyOn(api, "post").mockImplementation(
      async (_path, data) =>
        ({
          data: {
            changed: (data as { entity_ids: string[] }).entity_ids.length,
          },
        }) as never,
    );
    const ids = Array.from({ length: 501 }, (_, index) => `t${index}`);
    expect(await tagItems(7, "track", [...ids, "t0", ""])).toBe(501);
    expect(post).toHaveBeenCalledTimes(2);
    expect(post.mock.calls[0]?.[0]).toBe("/library/tags/7/items");
  });
});

describe("scoped focus", () => {
  test("composer and album scopes are fixed criteria, not chips", () => {
    const scope = [composerScope("c1", "Anna Hamilton")];
    let query = scopedFocus("albums", scope);
    expect(usesFocusEndpoint(query)).toBe(true);
    query = cycleFilter(query, pop);
    expect(query.scope).toEqual(scope);
    expect(query.filters).toHaveLength(1);
    // Switching browsers and clearing filters keep the scope.
    expect(withEntity(query, "songs").scope).toEqual(scope);
    const cleared = clearedFocus({ ...query, text: "allegro" });
    expect(cleared.filters).toEqual([]);
    expect(cleared.text).toBeNull();
    expect(cleared.scope).toEqual(scope);
    expect(facetBaseQuery({ ...query, match: "any" }).scope).toEqual(scope);
    expect(focusCountLabel(query, 2, 9)).toBe("2 of 9 albums");
    // URLs carry filters only; the page supplies its scope.
    expect(encodeFocus(query)).not.toContain("c1");
    expect(filterKey(albumScope("al-1"))).toBe("album:al-1");
    expect(criterionLabel(albumScope("al-1", "Box Set"))).toBe("Box Set");
  });

  test("credit groups and composers have Roon labels and sorts", () => {
    expect(facetTitle({ key: "credit", role: "conductor_ensemble" })).toBe(
      "Conductor / Ensemble",
    );
    expect(facetTitle({ key: "credit", role: "production" })).toBe(
      "Production",
    );
    expect(facetTitle({ key: "credit", role: "performer" })).toBe("Performers");
    expect(
      criterionLabel({
        facet: "credit",
        role: "conductor_ensemble",
        artist_id: "a",
        name: "Claudio Abbado",
      }),
    ).toBe("Claudio Abbado · Conductor / Ensemble");
    const composers = emptyFocus("composers");
    expect(effectiveSort(composers)).toBe("title");
    expect(usesFocusEndpoint(composers)).toBe(true);
    expect(entityNoun("composers", 2)).toBe("composers");
    expect(
      withEntity({ ...composers, sort: "track_count" }, "albums").sort,
    ).toBe("track_count");
    expect(withEntity({ ...composers, sort: "title" }, "songs").sort).toBe(
      "title",
    );
    const facets = visibleFacets(
      [
        {
          key: "credit",
          role: "composer",
          total: 1,
          values: [{ value: "c1", label: "Anna", count: 1 }],
        },
        {
          key: "credit",
          role: "conductor_ensemble",
          total: 1,
          values: [{ value: "c2", label: "Abbado", count: 1 }],
        },
      ],
      "composers",
    );
    expect(facets.map((facet) => facet.role)).toEqual(["conductor_ensemble"]);
    expect(
      decodeFocus(JSON.stringify({ filters: [albumScope("x")] }), "songs")
        .filters,
    ).toHaveLength(1);
  });

  test("compositions and people use the focus routes", async () => {
    const post = spyOn(api, "post").mockResolvedValue({
      data: { total: 0, trackTotal: 0, truncated: false, compositions: [] },
    } as never);
    const get = spyOn(api, "get").mockResolvedValue({
      data: { id: "a b", name: "A", roles: [], composer: null },
    } as never);
    const query = scopedFocus("songs", [composerScope("c1")]);
    await getFocusCompositions(query);
    expect(post).toHaveBeenCalledWith("/library/focus/compositions", {
      query,
    });
    await getFocusPerson("a b");
    expect(get).toHaveBeenCalledWith("/library/focus/people/a%20b");
  });
});
