import { describe, expect, test } from "bun:test";
import {
  clearUpcomingItems,
  removeUpcomingItem,
  shuffleUpcomingItems,
  unshuffleUpcomingItems,
} from "./player-queue-edit";

describe("local queue edits", () => {
  test("removes only upcoming items", () => {
    expect(removeUpcomingItem(["a", "b", "c", "d"], 1, 2)).toEqual([
      "a",
      "b",
      "d",
    ]);
    expect(removeUpcomingItem(["a", "b", "c"], 1, 1)).toBeNull();
    expect(removeUpcomingItem(["a", "b", "c"], 1, 0)).toBeNull();
    expect(removeUpcomingItem(["a", "b", "c"], 1, 3)).toBeNull();
  });

  test("clear upcoming keeps history and the current track", () => {
    expect(clearUpcomingItems(["a", "b", "c", "d"], 1)).toEqual(["a", "b"]);
  });

  test("shuffle leaves history and current in place and unshuffle restores", () => {
    const items = ["a", "b", "c", "d", "e", "f"];
    const shuffled = shuffleUpcomingItems(items, 1, () => 0);
    expect(shuffled.slice(0, 2)).toEqual(["a", "b"]);
    expect([...shuffled].sort()).toEqual(items);
    expect(shuffled).not.toEqual(items);
    const withAddition = [...shuffled, "g"];
    expect(unshuffleUpcomingItems(withAddition, 1, items)).toEqual([
      ...items,
      "g",
    ]);
  });
});
