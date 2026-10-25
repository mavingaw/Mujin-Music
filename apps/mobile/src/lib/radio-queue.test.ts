import { expect, test } from "bun:test";
import { radioBanIndex } from "@parson/music-sdk";
import {
  appendUnique,
  songBanned,
  withoutBannedRadioPicks,
  extendWithRadio,
  isCurrentRadioPick,
  needsRadioTopUp,
  radioStartIndex,
} from "./radio-queue";

const songs = (...ids: string[]) => ids.map((id) => ({ id }));

test("the Radio section is the trailing run of Radio picks", () => {
  const queue = songs("a", "b", "r1", "r2");
  const radio = new Set(["r1", "r2"]);
  expect(radioStartIndex(queue, radio, 0)).toBe(2);
  expect(radioStartIndex(queue, radio, 2)).toBe(3);
  expect(isCurrentRadioPick(queue, radio, 2)).toBe(true);
  expect(isCurrentRadioPick(queue, radio, 1)).toBe(false);
});

test("Radio tops up near the end of the queue only", () => {
  const radio = new Set(["r1"]);
  expect(needsRadioTopUp(songs("a", "b", "c", "d", "e"), radio, 0)).toBe(false);
  expect(needsRadioTopUp(songs("a", "b", "c"), radio, 0)).toBe(true);
  expect(
    needsRadioTopUp(
      songs("a", "r1", "r2", "r3"),
      new Set(["r1", "r2", "r3"]),
      0,
    ),
  ).toBe(false);
  expect(needsRadioTopUp([], radio, -1)).toBe(false);
});

test("extending with Radio keeps a bounded history and skips duplicates", () => {
  expect(
    appendUnique(songs("a"), songs("a", "b", "b")).map((s) => s.id),
  ).toEqual(["a", "b"]);
  const queue = songs("a", "b", "c", "d");
  const extended = extendWithRadio(queue, 3, songs("d", "e"), 2);
  expect(extended.queue.map((s) => s.id)).toEqual(["b", "c", "d", "e"]);
  expect(extended.nextIndex).toBe(3);
  expect(extended.queue[extended.nextIndex - 1]?.id).toBe("d");
});

test("a new ban drops upcoming Radio picks but keeps songs you queued", () => {
  const song = (id: string, artist: string, album = `album-${id}`) => ({
    id,
    artist_object: { id: artist },
    album_object: { id: album },
    contributing_artist_ids: [],
  });
  const queue = [
    song("m1", "banned"),
    song("r1", "banned"),
    song("r2", "ok"),
    song("r3", "ok", "banned-album"),
  ];
  const radio = new Set(["r1", "r2", "r3"]);
  const bans = radioBanIndex([
    { kind: "artist", id: "banned" },
    { kind: "album", id: "banned-album" },
  ]);
  expect(songBanned(bans, queue[0]!)).toBeTrue();

  const upcoming = withoutBannedRadioPicks(queue, radio, 0, bans);
  expect(upcoming.queue.map((item) => item.id)).toEqual(["m1", "r2"]);
  expect(upcoming.removedIds).toEqual(["r1", "r3"]);
  expect(upcoming.currentBanned).toBeFalse();

  const playing = withoutBannedRadioPicks(queue, radio, 1, bans);
  expect(playing.currentBanned).toBeTrue();
  expect(playing.queue.map((item) => item.id)).toEqual(["m1", "r1", "r2"]);
});
