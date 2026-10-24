import { expect, test } from "bun:test";
import {
  appendUnique,
  extendWithRadio,
  isCurrentRadioPick,
  needsRadioTopUp,
  radioStartIndex,
} from "./radio-queue";

const songs = (...ids: string[]) => ids.map((id) => ({ id }));

test("the Radio section is the trailing run of Radio picks", () => {
  const queue = songs("a", "b", "r1", "r2");
  const radio = new Set(["r1", "r2"]);
  expect(radioStartIndex(queue, radio, 0)).toBe(2);
  expect(radioStartIndex(queue, radio, 2)).toBe(3);
  expect(isCurrentRadioPick(queue, radio, 2)).toBe(true);
  expect(isCurrentRadioPick(queue, radio, 1)).toBe(false);
});

test("Radio tops up near the end of the queue only", () => {
  const radio = new Set(["r1"]);
  expect(needsRadioTopUp(songs("a", "b", "c", "d", "e"), radio, 0)).toBe(false);
  expect(needsRadioTopUp(songs("a", "b", "c"), radio, 0)).toBe(true);
  expect(
    needsRadioTopUp(
      songs("a", "r1", "r2", "r3"),
      new Set(["r1", "r2", "r3"]),
      0,
    ),
  ).toBe(false);
  expect(needsRadioTopUp([], radio, -1)).toBe(false);
});

test("extending with Radio keeps a bounded history and skips duplicates", () => {
  expect(
    appendUnique(songs("a"), songs("a", "b", "b")).map((s) => s.id),
  ).toEqual(["a", "b"]);
  const queue = songs("a", "b", "c", "d");
  const extended = extendWithRadio(queue, 3, songs("d", "e"), 2);
  expect(extended.queue.map((s) => s.id)).toEqual(["b", "c", "d", "e"]);
  expect(extended.nextIndex).toBe(3);
  expect(extended.queue[extended.nextIndex - 1]?.id).toBe("d");
});
