import { describe, expect, test } from "bun:test";
import type { ImportItem } from "@parson/music-sdk";
import {
  acceptAllSuggestions,
  defaultFilter,
  filterCounts,
  formatTrackDuration,
  includedSongIds,
  jobProgressLabel,
  skipAllUnmatched,
  spotifyRedirectAllowed,
  spotifyRedirectUri,
} from "./import-review-state";

function item(
  position: number,
  status: ImportItem["status"],
  songId: string | null,
): ImportItem {
  return {
    position,
    status,
    song_id: songId,
    confidence: 0.7,
    reasons: [],
    candidates: [],
    source: {
      title: `Track ${position}`,
      artists: [],
      album: null,
      duration_ms: null,
      isrc: null,
      location: null,
      external_id: null,
      external_url: null,
    },
  };
}

const items = [
  item(0, "matched", "a"),
  item(1, "review", "b"),
  item(2, "review", null),
  item(3, "unmatched", null),
  item(4, "confirmed", "a"),
  item(5, "skipped", "c"),
];

describe("import review", () => {
  test("counts every filter and starts where decisions are needed", () => {
    expect(filterCounts(items)).toEqual({
      review: 2,
      unmatched: 1,
      matched: 2,
      skipped: 1,
      all: 6,
    });
    expect(defaultFilter(items)).toBe("review");
    expect(defaultFilter([item(0, "unmatched", null)])).toBe("unmatched");
    expect(defaultFilter([item(0, "matched", "a")])).toBe("all");
  });

  test("bulk actions only touch the tracks they describe", () => {
    expect(acceptAllSuggestions(items)).toEqual([
      { action: "accept", position: 1 },
    ]);
    expect(skipAllUnmatched(items)).toEqual([{ action: "skip", position: 3 }]);
  });

  test("the saved playlist holds matched and confirmed songs once", () => {
    expect(includedSongIds({ items })).toEqual(["a"]);
  });

  test("progress labels explain each stage", () => {
    const base = {
      total: 10,
      matched: 6,
      review: 2,
      unmatched: 1,
      skipped: 1,
      fetched: null,
      error: null,
    };
    expect(jobProgressLabel({ ...base, status: "ready" } as never)).toBe(
      "6 of 9 matched · 2 to review · 1 not found",
    );
    expect(
      jobProgressLabel({ ...base, status: "fetching", fetched: 1200 } as never),
    ).toContain("1,200");
    expect(
      jobProgressLabel({ ...base, status: "failed", error: "Nope" } as never),
    ).toBe("Nope");
  });

  test("durations and Spotify redirect rules", () => {
    expect(formatTrackDuration(185_400)).toBe("3:05");
    expect(formatTrackDuration(null)).toBe("");
    expect(spotifyRedirectUri("http://127.0.0.1:1993/")).toBe(
      "http://127.0.0.1:1993/imports",
    );
    expect(spotifyRedirectAllowed("http://127.0.0.1:1993")).toBeTrue();
    expect(spotifyRedirectAllowed("https://music.example.com")).toBeTrue();
    expect(spotifyRedirectAllowed("http://localhost:1993")).toBeFalse();
    expect(spotifyRedirectAllowed("http://192.168.1.2:1993")).toBeFalse();
  });
});
