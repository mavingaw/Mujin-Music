import { expect, test } from "bun:test";
import { manualQueueItems, radioPickItems } from "./player-model";
import {
  appendRadioPicks,
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
