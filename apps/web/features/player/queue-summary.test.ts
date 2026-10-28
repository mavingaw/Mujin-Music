import { expect, test } from "bun:test";
import { queueRowDuration, queueSummary } from "./queue-summary";

const song = (duration: number) => ({ song: { duration } });

test("the queue header counts tracks and the time left from the current one", () => {
  expect(queueSummary([])).toBe("Nothing queued");
  expect(queueSummary([song(20)])).toBe("1 track");
  expect(queueSummary([song(600), song(1200), song(1800)], 1)).toBe(
    "3 tracks · 50 min left",
  );
  expect(queueSummary([song(3600), song(1500)], 0)).toBe(
    "2 tracks · 1 h 25 min left",
  );
});

test("queue rows show each track's length", () => {
  expect(queueRowDuration(187)).toBe("3:07");
  expect(queueRowDuration(35.2)).toBe("0:35");
  expect(queueRowDuration(3765)).toBe("1:02:45");
  expect(queueRowDuration(0)).toBe("");
  expect(queueRowDuration(undefined)).toBe("");
  expect(queueRowDuration(Number.NaN)).toBe("");
});
