import { expect, test } from "bun:test";
import { moveQueueItem, queueIndexAfterMove } from "./queue-edit";

test("moving queue items keeps the playing song selected", () => {
  expect(moveQueueItem(["a", "b", "c", "d"], 3, 1)).toEqual([
    "a",
    "d",
    "b",
    "c",
  ]);
  expect(moveQueueItem(["a", "b"], 0, 5)).toEqual(["a", "b"]);
  expect(queueIndexAfterMove(1, 3, 1)).toBe(2);
  expect(queueIndexAfterMove(1, 0, 2)).toBe(0);
  expect(queueIndexAfterMove(1, 2, 3)).toBe(1);
  expect(queueIndexAfterMove(1, 1, 3)).toBe(3);
});
