import { expect, test } from "bun:test";
import { radioBanIndex } from "@parson/music-sdk";
import { manualQueueItems, radioPickItems } from "./player-model";
import {
  appendRadioPicks,
  queueItemBanned,
  withoutBannedRadioPicks,
  markRadioPickLiked,
  radioExclusions,
  removeUpcomingItem,
  withoutRadioSection,
} from "./radio-queue";

const manual = manualQueueItems([
  { song: { id: "a", name: "A" } },
  { song: { id: "b", name: "B" } },
]);
const picks = radioPickItems([
  {
    id: "c",
    name: "C",
    artist: "Artist",
    contributing_artists: [],
    contributing_artist_ids: [],
    track_number: 1,
    duration: 200,
    album_object: { id: "al", name: "Album", cover_url: "" },
    artist_object: { id: "ar", name: "Artist", icon_url: "" },
    origin: "generated",
    queue_position: 0,
    reason: "from this artist",
  },
] as never);

test("radio picks become generated queue items that explain themselves", () => {
  expect(picks[0]?.origin).toBe("generated");
  expect(picks[0]?.reason).toBe("from this artist");
  expect(picks[0]?.queuePosition).toBeNull();
});

test("appending picks skips songs that are already queued", () => {
  const queue = appendRadioPicks(manual, [...picks, ...picks, manual[0]!]);
  expect(queue.map((item) => item.song.id)).toEqual(["a", "b", "c"]);
});

test("a song that already played can come back as a later Radio pick", () => {
  // Only what is queued from `fromIndex` on blocks a repeat.
  const queue = [...manual, ...picks];
  const again = radioPickItems([
    { ...(picks[0] as object), id: "a", name: "A again" },
    { ...(picks[0] as object), id: "c", name: "C again" },
  ] as never);
  expect(appendRadioPicks(queue, again, 3).map((item) => item.song.id)).toEqual(
    ["a", "b", "c", "a", "c"],
  );
  expect(appendRadioPicks(queue, again, 2).map((item) => item.song.id)).toEqual(
    ["a", "b", "c", "a"],
  );
});

test("removing an upcoming pick keeps the active item in place", () => {
  const queue = [...manual, ...picks];
  expect(removeUpcomingItem(queue, 1, 2)?.items.map((i) => i.song.id)).toEqual([
    "a",
    "b",
  ]);
  expect(removeUpcomingItem(queue, 1, 0)?.activeIndex).toBe(0);
  expect(removeUpcomingItem(queue, 1, 1)).toBeNull();
  expect(removeUpcomingItem(queue, 1, 9)).toBeNull();
});

test("switching Radio off drops only the Radio section", () => {
  const queue = [...manual, ...picks];
  expect(withoutRadioSection(queue, 2).map((i) => i.song.id)).toEqual([
    "a",
    "b",
  ]);
  expect(withoutRadioSection(queue, -1)).toHaveLength(3);
});

test("thumbs up marks a pick and exclusions cover rejected songs", () => {
  const queue = markRadioPickLiked([...manual, ...picks], 2);
  expect(queue[2]?.liked).toBe(true);
  expect(queue[0]?.liked).toBeUndefined();
  expect(radioExclusions(queue, ["z", "a"])).toEqual(["a", "b", "c", "z"]);
});

test("a new ban drops upcoming Radio picks but keeps what you queued", () => {
  const pick = (id: string, artist: string) =>
    radioPickItems([
      {
        id,
        name: id,
        artist,
        contributing_artists: [],
        contributing_artist_ids: [],
        track_number: 1,
        duration: 200,
        album_object: { id: `album-${id}`, name: "Album", cover_url: "" },
        artist_object: { id: artist, name: artist, icon_url: "" },
        origin: "generated",
        queue_position: 0,
        reason: "fits this station",
      },
    ] as never)[0]!;
  const queue = [
    ...manualQueueItems([
      { song: { id: "m1", name: "M1" }, artist: { id: "banned-artist" } },
    ]),
    pick("p1", "banned-artist"),
    pick("p2", "ok-artist"),
    pick("p3", "banned-artist"),
  ];
  const bans = radioBanIndex([{ kind: "artist", id: "banned-artist" }]);

  const upcoming = withoutBannedRadioPicks(queue, 0, bans);
  expect(upcoming.items.map((item) => item.song.id)).toEqual(["m1", "p2"]);
  expect(upcoming.removed).toBe(2);
  expect(upcoming.currentBanned).toBeFalse();

  const playing = withoutBannedRadioPicks(queue, 1, bans);
  expect(playing.items.map((item) => item.song.id)).toEqual(["m1", "p1", "p2"]);
  expect(playing.currentBanned).toBeTrue();
  expect(queueItemBanned(bans, queue[2]!)).toBeFalse();
  expect(withoutBannedRadioPicks(queue, 0, radioBanIndex([])).removed).toBe(0);
});
