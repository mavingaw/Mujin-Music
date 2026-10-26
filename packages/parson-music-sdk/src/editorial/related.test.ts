import { describe, expect, test } from "bun:test";
import type {
  InfluencePerson,
  PersonRelations,
  RelatedPerson,
} from "../domain/types";
import {
  collaboratorLine,
  influenceLine,
  influencesEmptyLabel,
  performanceLine,
  relatedSections,
} from "./related";

const person = (overrides: Partial<RelatedPerson> = {}): RelatedPerson => ({
  artist_id: "evans",
  name: "Bill Evans",
  image_url: null,
  roles: ["piano", "composer"],
  album_count: 2,
  track_count: 9,
  work_count: 4,
  albums: [],
  ...overrides,
});

const relations = (
  overrides: Partial<PersonRelations> = {},
): PersonRelations => ({
  artist_id: "miles",
  requested_id: "miles",
  collaborators: [],
  top_performers: [],
  performing_the_music_of: [],
  influences: [],
  followers: [],
  influence_source: null,
  enrichment_enabled: true,
  ...overrides,
});

describe("related people lines", () => {
  test("collaborators read their roles and what they share", () => {
    expect(collaboratorLine(person())).toBe(
      "Piano, composer · 2 albums · 9 tracks",
    );
    expect(
      collaboratorLine(
        person({ roles: ["artist"], album_count: 1, track_count: 1 }),
      ),
    ).toBe("Main artist · 1 album · 1 track");
    expect(collaboratorLine(person({ roles: [] }))).toBe("2 albums · 9 tracks");
  });

  test("top performers say what they are; composers what is performed", () => {
    expect(
      performanceLine(
        person({ kind: "conductor", album_count: 3, work_count: 2 }),
      ),
    ).toBe("Conductor · 3 recordings · 2 works");
    expect(
      performanceLine(
        person({ kind: "ensemble", album_count: 1, work_count: 1 }),
      ),
    ).toBe("Ensemble · 1 recording · 1 work");
    expect(performanceLine(person({ work_count: 1, album_count: 2 }))).toBe(
      "1 work · 2 albums",
    );
  });

  test("influences say whether they are in the library", () => {
    const influence: InfluencePerson = {
      artist_id: "debussy",
      name: "Claude Debussy",
      description: "French composer (1862–1918)",
      image_url: null,
      in_library: true,
      wikidata_url: "https://www.wikidata.org/wiki/Q4700",
    };
    expect(influenceLine(influence)).toBe(
      "In your library · French composer (1862–1918)",
    );
    expect(
      influenceLine({
        ...influence,
        in_library: false,
        description: "x".repeat(80),
      }),
    ).toBe(`X${"x".repeat(56)}…`);
    expect(
      influenceLine({ ...influence, in_library: false, description: null }),
    ).toBe("");
  });
});

describe("relatedSections", () => {
  test("lists only sections with people, composers first", () => {
    expect(relatedSections(null)).toEqual([]);
    expect(relatedSections(relations())).toEqual([]);
    expect(
      relatedSections(
        relations({
          collaborators: [person()],
          top_performers: [person({ kind: "soloist" })],
          followers: [
            {
              artist_id: null,
              name: "Herbie Hancock",
              description: null,
              image_url: null,
              in_library: false,
              wikidata_url: "https://www.wikidata.org/wiki/Q192474",
            },
          ],
        }),
      ),
    ).toEqual(["top_performers", "collaborators", "influences"]);
  });

  test("says why influences are missing", () => {
    expect(influencesEmptyLabel(relations())).toContain("Wikidata lists no");
    expect(
      influencesEmptyLabel(relations({ enrichment_enabled: false })),
    ).toContain("when online metadata is on");
  });
});
