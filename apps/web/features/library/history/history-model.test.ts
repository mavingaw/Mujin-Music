import { expect, test } from "bun:test";
import {
  bucketLabel,
  bucketTitle,
  clockOrder,
  fillBuckets,
  formatChange,
  labelEvery,
} from "./history-model";

test("bounded periods are zero-filled day by day", () => {
  const from = new Date(2026, 9, 1).toISOString();
  const to = new Date(2026, 9, 4).toISOString();
  const filled = fillBuckets(
    [{ bucket: "2026-10-02", plays: 3, listenedSeconds: 600 }],
    { from, to, bucket: "day" },
    "day",
  );
  expect(filled.map((item) => [item.bucket, item.plays])).toEqual([
    ["2026-10-01", 0],
    ["2026-10-02", 3],
    ["2026-10-03", 0],
  ]);
  const months = fillBuckets(
    [],
    {
      from: new Date(2026, 0, 1).toISOString(),
      to: new Date(2027, 0, 1).toISOString(),
      bucket: "month",
    },
    "month",
  );
  expect(months).toHaveLength(12);
  expect(months[11]?.bucket).toBe("2026-12");
  const open = [{ bucket: "2025-03", plays: 1, listenedSeconds: 1 }];
  expect(fillBuckets(open, { bucket: "month" }, "month")).toBe(open);
});

test("bucket labels read naturally", () => {
  expect(bucketLabel("2026-10-07", "day")).toBe("7");
  expect(bucketLabel("2026-03", "month")).toBe("Mar 26");
  expect(bucketLabel("2026-10-05", "week")).toBe("5 Oct");
  expect(bucketTitle("2026-03", "month")).toBe("March 2026");
  expect(bucketTitle("2026-10-05", "week")).toStartWith("Week of 5 Oct");
});

test("changes, clock order and tick spacing", () => {
  expect(formatChange(0.124)).toBe("+12%");
  expect(formatChange(-0.08)).toBe("−8%");
  expect(formatChange(0.001)).toBe("±0%");
  expect(formatChange(null)).toBeNull();
  const hours = Array.from({ length: 24 }, (_, hour) => hour);
  expect(clockOrder(hours)[0]).toEqual({ hour: 6, plays: 6 });
  expect(clockOrder(hours)[23]).toEqual({ hour: 5, plays: 5 });
  expect(labelEvery(30)).toBe(3);
  expect(labelEvery(5)).toBe(1);
});
