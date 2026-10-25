import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  clearAlbumIdentification,
  getArtistWorks,
  getSongWorks,
  getWork,
  identifyAlbum,
  searchAlbumReleases,
} from "./works";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("work reads use the work, artist and song routes with encoded ids", async () => {
  const get = spyOn(api, "get")
    .mockResolvedValueOnce(ok({ work: { mbid: "w" } }))
    .mockResolvedValueOnce(ok({ works: [] }))
    .mockResolvedValueOnce(ok({ song_id: "s", works: [{ track_id: "s" }] }));
  await getWork("w 1");
  await getArtistWorks("mb:1f9d");
  expect(await getSongWorks("a/b")).toEqual([{ track_id: "s" }] as never);
  expect(get.mock.calls.map((call) => call[0])).toEqual([
    "/works/w%201",
    "/artists/mb%3A1f9d/works",
    "/songs/a%2Fb/works",
  ]);
});

test("identify album searches, pins and clears through the admin routes", async () => {
  const get = spyOn(api, "get").mockResolvedValue(ok({ releases: [] }));
  const put = spyOn(api, "put").mockResolvedValue(
    ok({ identification: { release_mbid: "r" } }),
  );
  const del = spyOn(api, "delete").mockResolvedValue(ok(null));
  await searchAlbumReleases("album 1");
  await searchAlbumReleases("album 1", { query: "  karajan 1963 " });
  await searchAlbumReleases("album 1", {
    title: "Symphony No. 9",
    artist: " ",
  });
  expect(get.mock.calls.map((call) => call[0])).toEqual([
    "/metadata/identify?album_id=album+1",
    "/metadata/identify?album_id=album+1&query=karajan+1963",
    "/metadata/identify?album_id=album+1&title=Symphony+No.+9&artist=",
  ]);
  expect(
    (await identifyAlbum("a/1", { mbid: "r", title: "T", artist: "A" }))
      ?.release_mbid,
  ).toBe("r");
  expect(put).toHaveBeenCalledWith("/metadata/identify/a%2F1", {
    release_mbid: "r",
    title: "T",
    artist: "A",
  });
  await clearAlbumIdentification("a/1");
  expect(del).toHaveBeenCalledWith("/metadata/identify/a%2F1");
});
