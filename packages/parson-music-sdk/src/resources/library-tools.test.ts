import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  albumizeTracks,
  chooseAlbumVersion,
  forgetMissingFiles,
  getAlbumVersions,
  getSkippedPlaylist,
  preferTrackCopy,
  reanalyzeLibraryItems,
  resolveDuplicate,
  toolJobProgress,
  type ToolJob,
} from "./library-tools";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

const job = (overrides: Partial<ToolJob>): ToolJob => ({
  id: 1,
  kind: "reanalyze",
  title: "Re-analyze audio",
  scope: "Whole library",
  state: "running",
  total: 0,
  done: 0,
  failed: 0,
  current: "",
  message: "",
  started_at_ms: 0,
  ...overrides,
});

test("operations send a scope the backend reads", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok(job({})));
  await reanalyzeLibraryItems({ scope: "albums", ids: ["a 1"] }, true);
  expect(post).toHaveBeenCalledWith("/library/tools/reanalyze", {
    scope: "albums",
    ids: ["a 1"],
    force: true,
  });

  await resolveDuplicate("k", "merge", { keepSongId: "s" });
  expect(post).toHaveBeenLastCalledWith("/library/tools/duplicates/resolve", {
    key: "k",
    action: "merge",
    keep_song_id: "s",
    song_ids: undefined,
  });

  await preferTrackCopy("t", null);
  expect(post).toHaveBeenLastCalledWith("/library/tools/copies/prefer", {
    track_id: "t",
    path: null,
  });

  await chooseAlbumVersion("g", "album", true);
  expect(post).toHaveBeenLastCalledWith("/library/tools/versions/choice", {
    key: "g",
    preferred_album_id: "album",
    hide_others: true,
  });
});

test("albumizer, missing files and playlists use their endpoints", async () => {
  const post = spyOn(api, "post").mockImplementation(async (path) =>
    path === "/library/tools/albumizer"
      ? ok({ album_id: "ua-1" })
      : ok({ forgotten: 2 }),
  );
  expect(
    await albumizeTracks({
      title: "Demos",
      artist_name: "Someone",
      song_ids: ["b", "a"],
    }),
  ).toBe("ua-1");
  expect(await forgetMissingFiles(2)).toBe(2);
  expect(post).toHaveBeenLastCalledWith("/library/tools/missing/forget", {
    confirm: 2,
    paths: undefined,
  });

  const get = spyOn(api, "get").mockImplementation(async (path) =>
    path.endsWith("/versions")
      ? ok({ group: null })
      : ok({ file_name: "mix.m3u", content: "#EXTM3U" }),
  );
  expect(await getAlbumVersions("album/1")).toBeNull();
  expect(get).toHaveBeenCalledWith("/library/tools/albums/album%2F1/versions");
  expect((await getSkippedPlaylist("/m/mix.m3u")).file_name).toBe("mix.m3u");
  expect(get).toHaveBeenLastCalledWith("/library/tools/skipped/playlist", {
    params: { path: "/m/mix.m3u" },
  });
});

test("job progress is a fraction, indeterminate while the size is unknown", () => {
  expect(toolJobProgress(job({ total: 0 }))).toBeNull();
  expect(toolJobProgress(job({ total: 4, done: 1 }))).toBe(0.25);
  expect(toolJobProgress(job({ total: 4, done: 9 }))).toBe(1);
  expect(toolJobProgress(job({ state: "completed" }))).toBe(1);
});
