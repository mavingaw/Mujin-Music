import { expect, test } from "bun:test";
import type {
  AboutCredit,
  AlbumAbout,
  ReleaseMatch,
  ResolvedFact,
} from "../domain/types";
import {
  albumHeroYear,
  cleanEditedCredits,
  editableCredits,
  factSourceLabel,
  fieldChoice,
  filterIdentifyReleases,
  hasSourceOverrides,
  isTextField,
  resolvedFact,
  sourcesPatch,
} from "./sources";

const fact = (overrides: Partial<ResolvedFact>): ResolvedFact => ({
  field: "label",
  value: "Columbia",
  source: "tags",
  preferred: "tags",
  choice: null,
  library_default: "tags",
  tags: "Columbia",
  online: "CBS",
  edited: null,
  fallback: false,
  ...overrides,
});

const about = (facts: ResolvedFact[], heroYear: string | null = "1959") =>
  ({
    sources: {
      facts,
      hero_year: heroYear,
      credits_edited: false,
      edited_credits: null,
      online_release: true,
      online_edition: true,
      library: {},
    },
  }) as unknown as AlbumAbout;

test("facts say where they come from, including fallbacks and edits", () => {
  expect(factSourceLabel(fact({}))).toBe("File tags");
  expect(
    factSourceLabel(
      fact({ source: "online", fallback: true, value: "CBS", tags: null }),
    ),
  ).toBe("MusicBrainz (no file tags value)");
  expect(factSourceLabel(fact({ source: "edited" }))).toBe("Edited");
  expect(factSourceLabel(fact({ source: null, value: null }))).toBeNull();
  expect(fieldChoice(fact({}))).toBe("default");
  expect(fieldChoice(fact({ choice: "online" }))).toBe("online");
  expect(isTextField("catalog_number")).toBe(true);
  expect(isTextField("credits")).toBe(false);
});

test("the hero year is the original release, then the tag date", () => {
  const withSources = about([fact({})], "1959");
  expect(albumHeroYear(withSources, "1997-03-25")).toBe("1959");
  expect(albumHeroYear(about([fact({})], null), "1997-03-25")).toBe("1997");
  expect(albumHeroYear(undefined, "")).toBeNull();
  expect(resolvedFact(withSources, "label")?.value).toBe("Columbia");
  expect(resolvedFact(withSources, "country")).toBeNull();
});

test("an album with its own choices or frozen values says so", () => {
  expect(hasSourceOverrides(about([fact({})]))).toBe(false);
  expect(hasSourceOverrides(about([fact({ choice: "online" })]))).toBe(true);
  expect(hasSourceOverrides(about([fact({ edited: "Riverside" })]))).toBe(true);
  expect(hasSourceOverrides(undefined)).toBe(false);
});

test("credits become an editable list and are tidied before saving", () => {
  const credits: AboutCredit[] = [
    {
      artist_id: "evans",
      name: "Bill Evans",
      role: "performer",
      instrument: "piano",
      track_ids: ["t1", "t3"],
      sources: ["tags", "musicbrainz"],
    },
  ];
  const editable = editableCredits(credits);
  expect(editable).toEqual([
    {
      name: "Bill Evans",
      role: "performer",
      instrument: "piano",
      track_ids: ["t1", "t3"],
      artist_id: "evans",
    },
  ]);
  editable[0].track_ids.push("t2");
  expect(credits[0].track_ids).toEqual(["t1", "t3"]);
  expect(
    cleanEditedCredits([
      {
        name: "  Teo Macero ",
        role: "producer",
        instrument: "desk",
        track_ids: [],
      },
      { name: "   ", role: "composer", track_ids: [] },
      {
        name: "Paul Chambers",
        role: "performer",
        instrument: " Double Bass ",
        track_ids: [],
      },
    ]),
  ).toEqual([
    { name: "Teo Macero", role: "producer", instrument: null, track_ids: [] },
    {
      name: "Paul Chambers",
      role: "performer",
      instrument: "double bass",
      track_ids: [],
    },
  ]);
});

test("identify results can be narrowed to the tagged composer and conductor", () => {
  const release = (mbid: string, composer: boolean, conductor: boolean) =>
    ({
      mbid,
      credits_tagged_composer: composer,
      credits_tagged_conductor: conductor,
    }) as ReleaseMatch;
  const releases = [
    release("beethoven-kleiber", true, true),
    release("brahms-kleiber", false, true),
    release("beethoven-other", true, false),
  ];
  expect(
    filterIdentifyReleases(releases, { composer: true }).map((r) => r.mbid),
  ).toEqual(["beethoven-kleiber", "beethoven-other"]);
  expect(
    filterIdentifyReleases(releases, { composer: true, conductor: true }).map(
      (r) => r.mbid,
    ),
  ).toEqual(["beethoven-kleiber"]);
  expect(filterIdentifyReleases(releases, {})).toHaveLength(3);
});

test("a draft becomes the smallest patch, or none", () => {
  const facts = [
    fact({}),
    fact({ field: "catalog_number", edited: "CL 1355", source: "edited" }),
    fact({ field: "cover", choice: "online" }),
  ];
  expect(sourcesPatch(facts, { sources: {}, edits: {} })).toBeNull();
  expect(
    sourcesPatch(facts, {
      sources: { label: "default", cover: "online" },
      edits: { catalog_number: "CL 1355 " },
    }),
  ).toBeNull();
  expect(
    sourcesPatch(facts, {
      sources: { label: "online", cover: "default" },
      edits: { label: " Columbia ", catalog_number: "", cover: "x" },
    }),
  ).toEqual({
    sources: { label: "online", cover: null },
    edits: { label: "Columbia", catalog_number: null },
  });
});
