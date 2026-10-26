import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  artistEditorialImagePath,
  getAlbumAbout,
  getArtistAbout,
  getArtistCredits,
  getArtistRelated,
  getEnrichment,
  refreshEnrichment,
  saveNote,
  updateEnrichment,
} from "./editorial";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("editorial reads use the album and artist routes with encoded ids", async () => {
  const get = spyOn(api, "get").mockResolvedValue(ok({}));
  await getAlbumAbout("album/1");
  await getArtistAbout("mb:561d854a");
  await getArtistCredits("teo macero");
  await getArtistRelated("mb:561d854a");
  expect(get.mock.calls.map((call) => call[0])).toEqual([
    "/albums/album%2F1/about",
    "/artists/mb%3A561d854a/about",
    "/artists/teo%20macero/credits",
    "/artists/mb%3A561d854a/related",
  ]);
  expect(artistEditorialImagePath("a b")).toBe("/artists/a%20b/image");
});

test("notes are saved per entity and an empty body returns no note", async () => {
  const put = spyOn(api, "put")
    .mockResolvedValueOnce(
      ok({ note: { body: "First pressing", updated_at: "2026-10-24" } }),
    )
    .mockResolvedValueOnce(ok({ note: null }));
  expect((await saveNote("album", "x", "First pressing"))?.body).toBe(
    "First pressing",
  );
  expect(await saveNote("artist", "y", "")).toBeNull();
  expect(put.mock.calls).toEqual([
    ["/albums/x/note", { body: "First pressing" }],
    ["/artists/y/note", { body: "" }],
  ]);
});

test("enrichment settings use the administrator metadata routes", async () => {
  const get = spyOn(api, "get").mockResolvedValue(ok({ enabled: false }));
  const put = spyOn(api, "put").mockResolvedValue(ok({ enabled: true }));
  const post = spyOn(api, "post").mockResolvedValue(ok(null));
  expect((await getEnrichment()).enabled).toBeFalse();
  expect((await updateEnrichment({ enabled: true })).enabled).toBeTrue();
  await refreshEnrichment("artist", "miles");
  expect(get).toHaveBeenCalledWith("/metadata/enrichment");
  expect(put).toHaveBeenCalledWith("/metadata/enrichment", { enabled: true });
  expect(post).toHaveBeenCalledWith("/metadata/enrichment/refresh", {
    entity_type: "artist",
    entity_id: "miles",
  });
});
