import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  checkForFakeLossless,
  enhanceWithAi,
  fetchQueryPreview,
  getAlbumUpgradeStatus,
  isUpgradeOpen,
  requestLosslessVersions,
  saveUpgradeSecret,
  upgradeProgressDetail,
  upgradeStatusLabel,
  type UpgradeRequest,
} from "./upgrades";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

function request(overrides: Partial<UpgradeRequest>): UpgradeRequest {
  return {
    id: 1,
    album_id: "endgame",
    album_title: "Endgame",
    artist_name: "Rise Against",
    state: "downloading",
    message: "",
    origin: "manual",
    reason: "MP3 320",
    created_at: "2026-10-27 10:00:00",
    updated_at: "2026-10-27 10:01:00",
    progress: 45,
    speed: "3.2 MB/s",
    eta: "1m 10s",
    engine: "soulseek",
    jobs: [],
    result_album_id: null,
    ...overrides,
  };
}

test("status lines say what the owner asked for", () => {
  expect(upgradeStatusLabel(request({}))).toBe(
    "Lossless version on its way · 45%",
  );
  expect(upgradeStatusLabel(request({ progress: 0 }))).toBe(
    "Lossless version on its way",
  );
  expect(
    upgradeStatusLabel(request({ state: "handed_off", engine: "Lidarr" })),
  ).toBe("Handed to Lidarr");
  expect(upgradeStatusLabel(request({ state: "handed_off", engine: "" }))).toBe(
    "Handed to another app",
  );
  expect(upgradeStatusLabel(request({ state: "fake_lossless" }))).toContain(
    "original kept",
  );
  expect(upgradeProgressDetail(request({}))).toBe("3.2 MB/s · 1m 10s left");
  expect(isUpgradeOpen("handed_off")).toBeTrue();
  expect(isUpgradeOpen("upgraded")).toBeFalse();
});

test("the query preview matches the server's FETCH text", () => {
  expect(fetchQueryPreview({ artist: "Rise Against", title: "Endgame" })).toBe(
    "the album Endgame by Rise Against as flac",
  );
  expect(
    fetchQueryPreview({
      artist: "Rise Against",
      title: "Endgame",
      year: "2011-03-15",
      musicbrainzId: "6f1d0c3d-3f0e-4c37-9c8a-6c3bd1b1d1a7",
    }),
  ).toBe(
    "the album Endgame by Rise Against (2011, MusicBrainz release 6f1d0c3d-3f0e-4c37-9c8a-6c3bd1b1d1a7) as flac",
  );
});

test("requests, secrets and checks use the admin endpoints", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok({ requested: [4] }));
  expect(await requestLosslessVersions(["a 1"])).toEqual([4]);
  expect(post).toHaveBeenCalledWith("/upgrades/admin/requests", {
    album_ids: ["a 1"],
  });
  post.mockResolvedValue(ok({ queued: 3 }));
  expect(await enhanceWithAi({ albumIds: ["a"] })).toBe(3);
  expect(post).toHaveBeenLastCalledWith("/upgrades/admin/enhance", {
    album_ids: ["a"],
    song_ids: [],
  });
  await checkForFakeLossless(["a"]);
  expect(post).toHaveBeenLastCalledWith("/upgrades/admin/lossless-check", {
    album_ids: ["a"],
    force: false,
  });
  const put = spyOn(api, "put").mockResolvedValue(ok({ saved: true }));
  await saveUpgradeSecret("fetch_token", "token-value");
  expect(put).toHaveBeenCalledWith("/upgrades/admin/secrets/fetch_token", {
    value: "token-value",
  });
  const get = spyOn(api, "get").mockResolvedValue(
    ok({
      album_id: "a/b",
      request: null,
      enhanced_tracks: 0,
      enhance_queued: 0,
    }),
  );
  await getAlbumUpgradeStatus("a/b");
  expect(get).toHaveBeenCalledWith("/upgrades/albums/a%2Fb");
});
