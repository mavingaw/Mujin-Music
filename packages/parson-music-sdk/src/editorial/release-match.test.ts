import { describe, expect, test } from "bun:test";
import type { ReleaseMatchCheck } from "../domain/types";
import {
  releaseMatchFact,
  releaseYearDiffers,
  releaseMatchSignals,
  releaseMatchSummary,
  trackCountWarning,
  UNVERIFIED_MATCH_TEXT,
  unverifiedMatch,
  unverifiedMatchText,
} from "./release-match";

const single: ReleaseMatchCheck = {
  status: "unverified",
  method: "automatic",
  release_mbid: "11365fd1-0000-4000-8000-000000000002",
  release_title: "Blue Train",
  release_artist: "John Coltrane",
  url: "https://musicbrainz.org/release/11365fd1-0000-4000-8000-000000000002",
  score: -3,
  threshold: 3,
  passed: false,
  release_date: "1957",
  release_type: "Single",
  release_track_count: 2,
  local_track_count: 5,
  signals: [
    {
      key: "tracks",
      outcome: "mismatch",
      points: -3,
      detail: "The release has 2 tracks; your album has 5.",
    },
    {
      key: "lengths",
      outcome: "unknown",
      points: 0,
      detail: "Track lengths could not be compared.",
    },
    {
      key: "year",
      outcome: "match",
      points: 1,
      detail: "Released in 1957, as your tags say.",
    },
  ],
};

describe("release match check", () => {
  test("an unverified match is surfaced with its release and evidence", () => {
    expect(unverifiedMatch({ release_match: single })).toBe(single);
    expect(
      unverifiedMatch({ release_match: { ...single, status: "verified" } }),
    ).toBeNull();
    expect(releaseMatchSummary(single)).toBe("1957 · Single · 2 tracks");
    expect(releaseMatchSignals(single).map((signal) => signal.key)).toEqual([
      "tracks",
      "year",
      "lengths",
    ]);
  });

  test("the Matched fact says how the release was chosen", () => {
    expect(
      releaseMatchFact({ release_match: single, identification: null }),
    ).toBe("Unverified");
    expect(
      releaseMatchFact({
        release_match: { ...single, status: "verified", method: "tagged" },
        identification: null,
      }),
    ).toBe("Release id in your tags");
    expect(
      releaseMatchFact({
        release_match: {
          ...single,
          status: "verified",
          signals: [
            { key: "tracks", outcome: "match", points: 3, detail: "" },
            { key: "ensemble", outcome: "match", points: 2, detail: "" },
          ],
        },
        identification: null,
      }),
    ).toBe("Automatically · 2 of 2 checks agree");
    expect(releaseMatchFact(undefined)).toBeNull();
    expect(releaseYearDiffers({ release_match: single })).toBe(false);
    expect(
      releaseYearDiffers({
        release_match: {
          ...single,
          signals: [
            { key: "year", outcome: "mismatch", points: -2, detail: "" },
          ],
        },
      }),
    ).toBe(true);
  });

  test("choosing a release with another track count is warned about", () => {
    expect(trackCountWarning({ track_count: 19 }, 4)).toBe(
      "This release has 19 tracks; your album has 4. Tracks are paired by title, so some may get the wrong credits or piece.",
    );
    expect(trackCountWarning({ track_count: 4 }, 4)).toBeNull();
    expect(trackCountWarning({ track_count: null }, 4)).toBeNull();
  });
});

test("a partial copy of an album with an article says the album's text still applies", () => {
  expect(unverifiedMatchText(null)).toBe(UNVERIFIED_MATCH_TEXT);
  expect(unverifiedMatchText({ background: null, review: null })).toBe(
    UNVERIFIED_MATCH_TEXT,
  );
  const text = unverifiedMatchText({
    background: null,
    review: {
      kind: "reception",
      source: "wikipedia",
      language: "en",
      title: "The Slip (album)",
      body: "Critical reception.",
      url: "https://en.wikipedia.org/wiki/The_Slip_(album)",
      license: "CC BY-SA 4.0",
      license_url: "https://creativecommons.org/licenses/by-sa/4.0/",
    },
  });
  expect(text).toContain("background and reception");
  expect(text).not.toContain("release type.");
});
