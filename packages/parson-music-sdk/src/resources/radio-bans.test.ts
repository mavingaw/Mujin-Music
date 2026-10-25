import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  banFromRadio,
  clearRadioDislike,
  getRadioBans,
  getRadioDislikes,
  unbanFromRadio,
} from "./radio-bans";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("bans use the playback radio endpoints", async () => {
  const get = spyOn(api, "get").mockResolvedValue(
    ok({ items: [{ kind: "artist", id: "a", name: "A", created_at: "" }] }),
  );
  expect(await getRadioBans()).toEqual([
    { kind: "artist", id: "a", name: "A", created_at: "" },
  ]);
  expect(get).toHaveBeenCalledWith("/playback/radio/bans");

  const post = spyOn(api, "post").mockResolvedValue(
    ok({ kind: "album", id: "x", name: "X", created_at: "" }),
  );
  await banFromRadio({ kind: "album", id: "x" });
  expect(post).toHaveBeenCalledWith("/playback/radio/bans", {
    kind: "album",
    id: "x",
  });

  const remove = spyOn(api, "delete").mockResolvedValue(ok(null));
  await unbanFromRadio({ kind: "song", id: "folder/song #1" });
  expect(remove).toHaveBeenCalledWith(
    "/playback/radio/bans/song/folder%2Fsong%20%231",
  );
});

test("dislikes can be listed and undone one at a time", async () => {
  const get = spyOn(api, "get").mockResolvedValue(
    ok({ items: [{ song: { id: "s1" } }] }),
  );
  expect(await getRadioDislikes()).toEqual([{ song: { id: "s1" } }] as never);
  expect(get).toHaveBeenCalledWith("/playback/radio/dislikes");

  const remove = spyOn(api, "delete").mockResolvedValue(ok(null));
  await clearRadioDislike("s 1");
  expect(remove).toHaveBeenCalledWith("/playback/radio/dislikes/s%201");
});
