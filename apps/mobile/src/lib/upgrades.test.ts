import { expect, test } from "bun:test";
import type { AlbumUpgradeStatus, UpgradeRequest } from "@parson/music-sdk";
import { upgradeChips } from "./upgrades";

const request = (overrides: Partial<UpgradeRequest>): UpgradeRequest => ({
  id: 1,
  album_id: "a",
  album_title: "Endgame",
  artist_name: "Rise Against",
  state: "downloading",
  message: "",
  origin: "manual",
  reason: "",
  created_at: "",
  updated_at: "",
  progress: 45,
  speed: "",
  eta: "",
  engine: "",
  jobs: [],
  result_album_id: null,
  ...overrides,
});

const status = (
  overrides: Partial<AlbumUpgradeStatus>,
): AlbumUpgradeStatus => ({
  album_id: "a",
  request: null,
  track_count: 10,
  enhanced_tracks: 0,
  enhanced_copies: 0,
  enhance_queued: 0,
  ...overrides,
});

test("the album line shows a download on its way and AI copies", () => {
  expect(upgradeChips(undefined)).toEqual([]);
  const chips = upgradeChips(
    status({ request: request({}), enhanced_tracks: 3, enhanced_copies: 3 }),
  );
  expect(chips.map((chip) => chip.label)).toEqual([
    "Lossless version on its way · 45%",
    "AI-enhanced · 3 of 10 tracks",
  ]);
  expect(chips[0]?.busy).toBeTrue();
  const handed = upgradeChips(
    status({ request: request({ state: "handed_off", engine: "Lidarr" }) }),
  );
  expect(handed[0]?.label).toBe("Handed to Lidarr");
  expect(handed[0]?.busy).toBeFalse();
  expect(
    upgradeChips(status({ request: request({ state: "cancelled" }) })),
  ).toEqual([]);
});
