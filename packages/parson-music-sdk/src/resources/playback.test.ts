import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api, { ApiError } from "../core/http";
import {
  getPlaybackQueueRevisionConflict,
  getRadioPicks,
  getRadioSettings,
  radioExhaustedMessage,
  radioExhaustedNotice,
  radioPicksExhaustedNotice,
  radioSeedParams,
  updateRadioSettings,
} from "./playback";

afterEach(() => {
  mock.restore();
});

test("radio seeds map to one request field per kind", () => {
  expect(radioSeedParams({ kind: "song", id: "s" })).toEqual({
    seed_song_id: "s",
  });
  expect(radioSeedParams({ kind: "album", id: "a" })).toEqual({
    seed_album_id: "a",
  });
  expect(radioSeedParams({ kind: "artist", id: "r" })).toEqual({
    seed_artist_id: "r",
  });
  expect(radioSeedParams({ kind: "genre", id: "Jazz" })).toEqual({
    seed_genre: "Jazz",
  });
  expect(radioSeedParams(null)).toEqual({});
});

test("radio picks and settings use the playback radio endpoints", async () => {
  const post = spyOn(api, "post").mockResolvedValue({
    data: { items: [] },
    status: 200,
    headers: new Headers(),
  } as never);
  const exclusions = Array.from({ length: 600 }, (_, index) => `s${index}`);
  await getRadioPicks({
    seed: { kind: "genre", id: "Jazz" },
    exclude_song_ids: exclusions,
    limit: 1,
  });
  expect(post).toHaveBeenCalledWith("/playback/radio/picks", {
    seed_genre: "Jazz",
    exclude_song_ids: exclusions.slice(-500),
    limit: 1,
  });

  const get = spyOn(api, "get").mockResolvedValue({
    data: { enabled: false, coming_up_notice: true },
    status: 200,
    headers: new Headers(),
  } as never);
  expect(await getRadioSettings()).toEqual({
    enabled: false,
    coming_up_notice: true,
  });
  expect(get).toHaveBeenCalledWith("/playback/radio/settings", {
    timeout: 4_000,
  });
  const put = spyOn(api, "put").mockResolvedValue({
    data: { enabled: true, coming_up_notice: false },
    status: 200,
    headers: new Headers(),
  } as never);
  await updateRadioSettings({ coming_up_notice: false });
  expect(put).toHaveBeenCalledWith("/playback/radio/settings", {
    coming_up_notice: false,
  });
});

test("queue revision conflicts expose a validated reconciliation state", () => {
  const conflict = new ApiError(
    "conflict",
    {},
    {
      status: 409,
      headers: new Headers(),
      data: {
        error: "queue_revision_conflict",
        revision: 7,
        current_position: 3,
      },
    },
  );
  expect(getPlaybackQueueRevisionConflict(conflict)).toEqual({
    revision: 7,
    current_position: 3,
  });
  expect(
    getPlaybackQueueRevisionConflict(
      new ApiError(
        "bad response",
        {},
        {
          status: 409,
          headers: new Headers(),
          data: { error: "queue_revision_conflict", revision: -1 },
        },
      ),
    ),
  ).toBeNull();
});

test("radio picks send rejections apart from the recyclable queue", async () => {
  const post = spyOn(api, "post").mockResolvedValue({
    data: { items: [] },
    status: 200,
    headers: new Headers(),
  } as never);
  await getRadioPicks({
    seed: { kind: "song", id: "s1" },
    exclude_song_ids: ["s1", "s2"],
    reject_song_ids: ["s3"],
    limit: 6,
  });
  expect(post).toHaveBeenCalledWith("/playback/radio/picks", {
    seed_song_id: "s1",
    exclude_song_ids: ["s1", "s2"],
    reject_song_ids: ["s3"],
    limit: 6,
  });
});

test("an exhausted Radio is reported, not silently stopped", () => {
  const exhausted = new ApiError(
    "unprocessable",
    {},
    {
      status: 422,
      headers: new Headers(),
      data: { error: "radio_exhausted", message: "Nothing left." },
    },
  );
  expect(radioExhaustedNotice(exhausted)).toBe("Nothing left.");
  expect(
    radioExhaustedNotice(
      new ApiError(
        "unprocessable",
        {},
        {
          status: 422,
          headers: new Headers(),
          data: { error: "radio_exhausted" },
        },
      ),
    ),
  ).toBe(radioExhaustedMessage);
  expect(
    radioExhaustedNotice(
      new ApiError(
        "bad",
        {},
        { status: 400, headers: new Headers(), data: { error: "invalid" } },
      ),
    ),
  ).toBeNull();
  expect(radioExhaustedNotice(new Error("offline"))).toBeNull();
  expect(radioPicksExhaustedNotice({ items: [], exhausted: true })).toBe(
    radioExhaustedMessage,
  );
  expect(radioPicksExhaustedNotice({ items: [] })).toBeNull();
});
