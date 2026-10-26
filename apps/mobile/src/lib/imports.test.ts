import { expect, test } from "bun:test";
import type { ImportItem } from "@parson/music-sdk";

import {
  acceptSuggestionActions,
  confidencePercent,
  importStatusLine,
  initialMobileFilter,
  isJobRunning,
  itemsForFilter,
} from "./imports";

function item(
  position: number,
  status: ImportItem["status"],
  songId: string | null,
) {
  return {
    position,
    status,
    song_id: songId,
    confidence: 0.6,
    reasons: [],
    candidates: [],
    source: {
      title: "t",
      artists: [],
      album: null,
      duration_ms: null,
      isrc: null,
      location: null,
      external_id: null,
      external_url: null,
    },
  } satisfies ImportItem;
}

test("mobile review starts with tracks that need a decision", () => {
  const items = [item(0, "matched", "a"), item(1, "unmatched", null)];
  expect(initialMobileFilter(items)).toBe("unmatched");
  expect(initialMobileFilter([...items, item(2, "review", "b")])).toBe(
    "review",
  );
  expect(initialMobileFilter([item(0, "matched", "a")])).toBe("all");
  expect(itemsForFilter(items, "unmatched").map((i) => i.position)).toEqual([
    1,
  ]);
});

test("accepting suggestions only touches reviewable matches", () => {
  expect(
    acceptSuggestionActions([
      item(0, "review", "a"),
      item(1, "review", null),
      item(2, "matched", "b"),
    ]),
  ).toEqual([{ action: "accept", position: 0 }]);
});

test("status lines and progress", () => {
  const job = {
    status: "ready",
    total: 12,
    skipped: 2,
    matched: 7,
    review: 2,
    unmatched: 1,
    fetched: null,
    error: null,
  } as never;
  expect(importStatusLine(job)).toBe("7/10 matched · 2 to check · 1 not found");
  expect(isJobRunning({ status: "matching" })).toBeTrue();
  expect(isJobRunning({ status: "ready" })).toBeFalse();
  expect(confidencePercent(0.876)).toBe("88%");
});
