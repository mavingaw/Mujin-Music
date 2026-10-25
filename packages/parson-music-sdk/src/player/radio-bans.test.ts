import { expect, test } from "bun:test";
import {
  applyRadioBanChange,
  filterRadioBans,
  heartState,
  isRadioBanned,
  isSongExcludedByBans,
  nextHeartState,
  radioBanActionLabel,
  radioBanIndex,
  withoutBannedSongs,
} from "./radio-bans";

const index = radioBanIndex([
  { kind: "song", id: "s1" },
  { kind: "album", id: "al1" },
  { kind: "artist", id: "ar1" },
]);

test("a ban covers the song, its album and every credited artist", () => {
  expect(isSongExcludedByBans(index, { songId: "s1" })).toBeTrue();
  expect(isSongExcludedByBans(index, { songId: "s2", albumId: "al1" })).toBe(
    true,
  );
  expect(
    isSongExcludedByBans(index, {
      songId: "s3",
      albumId: "al2",
      artistIds: ["ar2", "ar1"],
    }),
  ).toBeTrue();
  expect(
    isSongExcludedByBans(index, {
      songId: "s3",
      albumId: "al2",
      artistIds: ["ar2", null],
    }),
  ).toBeFalse();
  expect(isRadioBanned(index, { kind: "album", id: "al1" })).toBeTrue();
  expect(isRadioBanned(index, { kind: "song", id: "al1" })).toBeFalse();
});

test("shuffle drops banned songs and keeps order", () => {
  const songs = [
    { id: "s1", album: "al9", artist: "ar9" },
    { id: "s2", album: "al9", artist: "ar9" },
    { id: "s3", album: "al1", artist: "ar9" },
    { id: "s4", album: "al9", artist: "ar1" },
    { id: "s5", album: "al9", artist: "ar9" },
  ];
  const kept = withoutBannedSongs(songs, index, (song) => ({
    songId: song.id,
    albumId: song.album,
    artistIds: [song.artist],
  }));
  expect(kept.map((song) => song.id)).toEqual(["s2", "s5"]);
  expect(withoutBannedSongs(songs, radioBanIndex([]), () => ({})).length).toBe(
    5,
  );
});

test("the heart cycles none, liked, banned like Roon's", () => {
  expect(nextHeartState("none")).toBe("liked");
  expect(nextHeartState("liked")).toBe("banned");
  expect(nextHeartState("banned")).toBe("none");
  expect(heartState(true, true)).toBe("banned");
  expect(heartState(true, false)).toBe("liked");
  expect(heartState(false, false)).toBe("none");
});

test("ban labels, optimistic changes and the Banned filter", () => {
  expect(radioBanActionLabel("song", false)).toBe("Ban this track");
  expect(radioBanActionLabel("artist", true)).toBe("Unban this artist");
  const bans = applyRadioBanChange(
    [],
    { kind: "artist", id: "a", name: "A" },
    true,
  );
  expect(bans).toHaveLength(1);
  const more = applyRadioBanChange(bans, { kind: "song", id: "s" }, true);
  expect(more.map((ban) => ban.id)).toEqual(["s", "a"]);
  expect(applyRadioBanChange(more, { kind: "artist", id: "a" }, false)).toEqual(
    [more[0]!],
  );
  expect(filterRadioBans(more, "artist").map((ban) => ban.id)).toEqual(["a"]);
  expect(filterRadioBans(more, "all")).toHaveLength(2);
});
