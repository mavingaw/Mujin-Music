import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  getLibrarySources,
  revertAlbumCredits,
  saveAlbumCredits,
  updateAlbumSources,
  updateLibrarySources,
} from "./sources";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("album sources and credits go through the admin metadata routes", async () => {
  const put = spyOn(api, "put")
    .mockResolvedValueOnce(ok({ album_id: "a/1" }))
    .mockResolvedValueOnce(
      ok({
        credits: [{ name: "Bill Evans", role: "performer", track_ids: [] }],
      }),
    );
  const del = spyOn(api, "delete").mockResolvedValue(ok(null));
  await updateAlbumSources("a/1", { sources: { label: "online" } });
  const saved = await saveAlbumCredits("a/1", [
    { name: "Bill Evans", role: "performer", track_ids: [] },
  ]);
  await revertAlbumCredits("a/1");
  expect(put.mock.calls[0]).toEqual([
    "/metadata/albums/a%2F1/sources",
    { sources: { label: "online" }, edits: {} },
  ]);
  expect(put.mock.calls[1][0]).toBe("/metadata/albums/a%2F1/credits");
  expect(saved[0].name).toBe("Bill Evans");
  expect(del).toHaveBeenCalledWith("/metadata/albums/a%2F1/credits");
});

test("library defaults are read and changed field by field", async () => {
  const get = spyOn(api, "get").mockResolvedValue(ok({ label: "tags" }));
  const put = spyOn(api, "put").mockResolvedValue(ok({ label: "online" }));
  expect((await getLibrarySources()).label).toBe("tags");
  expect((await updateLibrarySources({ label: "online" })).label).toBe(
    "online",
  );
  expect(get).toHaveBeenCalledWith("/metadata/sources");
  expect(put).toHaveBeenCalledWith("/metadata/sources", { label: "online" });
});
