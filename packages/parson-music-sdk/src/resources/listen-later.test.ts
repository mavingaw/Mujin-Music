import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  addToListenLater,
  addedAgo,
  getListenLater,
  getListenLaterSaved,
  listenLaterLabels,
  removeFromListenLater,
  setListenLaterRemoveWhenPlayed,
  type ListenLaterItem,
} from "./listen-later";

afterEach(() => {
  mock.restore();
});

const response = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("listen later pages send only meaningful filters", async () => {
  const get = spyOn(api, "get").mockResolvedValue(response({}));
  const page = await getListenLater({
    type: "all",
    sort: "artist",
    direction: "asc",
    q: "  miles ",
  });
  expect(get).toHaveBeenCalledWith("/library/listen-later", {
    params: { sort: "artist", direction: "asc", q: "miles" },
  });
  expect(page).toEqual({
    total: 0,
    offset: 0,
    counts: { all: 0, album: 0, track: 0, artist: 0 },
    unavailable: 0,
    removeWhenPlayed: false,
    items: [],
  });

  await getListenLater({ type: "album" });
  expect(get).toHaveBeenLastCalledWith("/library/listen-later", {
    params: { type: "album" },
  });
});

test("adding and removing batch unique ids", async () => {
  const post = spyOn(api, "post").mockImplementation(async (path) =>
    response(
      String(path).endsWith("/remove") ? { removed: 2 } : { added: 501 },
    ),
  );
  const ids = Array.from({ length: 501 }, (_, index) => `a${index}`);
  expect(await addToListenLater("album", [...ids, "a0", ""])).toBe(1002);
  expect(post).toHaveBeenCalledTimes(2);
  expect(post.mock.calls[0]?.[1]).toEqual({
    entity_type: "album",
    entity_ids: ids.slice(0, 500),
  });
  expect(post.mock.calls[1]?.[1]).toEqual({
    entity_type: "album",
    entity_ids: ["a500"],
  });
  expect(await removeFromListenLater("track", ["t1", "t2"])).toBe(2);
  expect(post).toHaveBeenLastCalledWith("/library/listen-later/items/remove", {
    entity_type: "track",
    entity_ids: ["t1", "t2"],
  });
});

test("saved lookups and settings use the bounded endpoints", async () => {
  const post = spyOn(api, "post").mockResolvedValue(
    response({ ids: ["al-1"] }),
  );
  const saved = await getListenLaterSaved("album", ["al-1", "al-2"]);
  expect([...saved]).toEqual(["al-1"]);
  expect(post).toHaveBeenCalledWith(
    "/library/listen-later/saved",
    { entity_type: "album", entity_ids: ["al-1", "al-2"] },
    { timeout: 4_000 },
  );
  const put = spyOn(api, "put").mockResolvedValue(
    response({ remove_when_played: true }),
  );
  expect(await setListenLaterRemoveWhenPlayed(true)).toBeTrue();
  expect(put).toHaveBeenCalledWith("/library/listen-later/settings", {
    remove_when_played: true,
  });
});

test("labels describe each kind of saved item", () => {
  const album: ListenLaterItem = {
    entityType: "album",
    entityId: "al",
    addedAt: "2026-10-01T10:00:00Z",
    album: {
      id: "al",
      name: "Kind of Blue",
      artistId: "ar",
      artistName: "Miles Davis",
      coverPath: "",
      releaseYear: "1959",
      songCount: 5,
      quality: "hires",
    },
  };
  expect(listenLaterLabels(album)).toEqual({
    title: "Kind of Blue",
    subtitle: "Miles Davis · 1959",
  });
  expect(
    listenLaterLabels({
      entityType: "artist",
      entityId: "ar",
      addedAt: "",
      artist: {
        id: "ar",
        name: "Nina Simone",
        artworkPath: "",
        albumCount: 1,
        appearanceCount: 0,
        songCount: 9,
      },
    }).subtitle,
  ).toBe("Artist · 1 album");
});

test("added dates read naturally", () => {
  const now = new Date(2026, 9, 27, 15, 0);
  expect(addedAgo(new Date(2026, 9, 27, 9, 0).toISOString(), now)).toBe(
    "Added today",
  );
  expect(addedAgo(new Date(2026, 9, 26, 23, 0).toISOString(), now)).toBe(
    "Added yesterday",
  );
  expect(addedAgo(new Date(2026, 9, 23, 9, 0).toISOString(), now)).toBe(
    "Added 4 days ago",
  );
  expect(addedAgo(new Date(2026, 2, 12).toISOString(), now)).toBe(
    "Added 12 Mar",
  );
  expect(addedAgo(new Date(2025, 2, 12).toISOString(), now)).toBe(
    "Added 12 Mar 2025",
  );
  expect(addedAgo("not a date", now)).toBe("");
});
