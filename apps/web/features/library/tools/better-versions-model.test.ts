import { expect, test } from "bun:test";
import type { UpgradeCandidate, UpgradeRequest } from "@parson/music-sdk";
import {
  canRequest,
  matchesFilter,
  refreshInterval,
  requestableSelection,
} from "./better-versions-model";

function request(state: UpgradeRequest["state"]): UpgradeRequest {
  return {
    id: 1,
    album_id: "a",
    album_title: "A",
    artist_name: "X",
    state,
    message: "",
    origin: "manual",
    reason: "",
    created_at: "",
    updated_at: "",
    progress: null,
    speed: "",
    eta: "",
    engine: "",
    jobs: [],
    result_album_id: null,
  };
}

function candidate(
  id: string,
  overrides: Partial<UpgradeCandidate> = {},
): UpgradeCandidate {
  return {
    album_id: id,
    title: id,
    artist_id: "x",
    artist_name: "X",
    year: "2011",
    cover_url: "",
    musicbrainz_id: "",
    track_count: 10,
    quality: { tier: "lossy", score: 1, label: "MP3 320", format: "MP3" },
    reasons: [],
    request: null,
    enhanced_tracks: 0,
    enhance_queued: 0,
    search: null,
    ...overrides,
  };
}

test("albums with a request on its way cannot be requested again", () => {
  const waiting = candidate("waiting");
  const moving = candidate("moving", { request: request("downloading") });
  const failed = candidate("failed", { request: request("failed") });
  expect(canRequest(waiting)).toBeTrue();
  expect(canRequest(moving)).toBeFalse();
  expect(canRequest(failed)).toBeTrue();
  expect(
    requestableSelection(
      [waiting, moving, failed],
      new Set(["waiting", "moving"]),
    ).map((item) => item.album_id),
  ).toEqual(["waiting"]);
});

test("filters split waiting, moving and unfound albums", () => {
  const waiting = candidate("waiting");
  const handed = candidate("handed", { request: request("handed_off") });
  const unfound = candidate("unfound", { request: request("not_found") });
  const searched = candidate("searched", {
    search: { found: false, count: 0, titles: [] },
  });
  const all = [waiting, handed, unfound, searched];
  const ids = (filter: Parameters<typeof matchesFilter>[1]) =>
    all.filter((item) => matchesFilter(item, filter)).map((i) => i.album_id);
  expect(ids("all")).toHaveLength(4);
  expect(ids("waiting")).toEqual(["waiting", "searched"]);
  expect(ids("requested")).toEqual(["handed"]);
  expect(ids("unfound")).toEqual(["unfound", "searched"]);
});

test("the page refreshes quickly only while something moves", () => {
  expect(refreshInterval([candidate("a")], [request("upgraded")])).toBeFalse();
  expect(refreshInterval([], [request("downloading")])).toBe(4_000);
  expect(refreshInterval([candidate("a", { enhance_queued: 2 })], [])).toBe(
    4_000,
  );
});
