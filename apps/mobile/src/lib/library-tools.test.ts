import { expect, test } from "bun:test";
import type { SkippedFile, ToolJob } from "@parson/music-sdk";
import {
  formatBytes,
  formatPlayingTime,
  jobLine,
  losslessPercent,
  skippedByReason,
  tierColor,
} from "./library-tools";

test("library numbers read naturally", () => {
  expect(formatBytes(45_478_562)).toBe("45.5 MB");
  expect(formatPlayingTime(90_000)).toBe("1 d 1 hr");
  expect(
    losslessPercent([
      { key: "hi_res", label: "", tracks: 3 },
      { key: "lossy", label: "", tracks: 1 },
    ]),
  ).toBe(75);
  const palette = {
    accent: "#d98b5f",
    accentSoft: "#e6a27a",
    accentDeep: "#bf7246",
    qualityLow: "#7e7a74",
    muted: "#5c5955",
  };
  expect(tierColor("nope", palette)).toBe(tierColor("unknown", palette));
  expect(tierColor("hi_res", palette)).toBe("#d98b5f");
});

test("skipped files group by reason without ignored ones", () => {
  const file = (reason: string, ignored = false) =>
    ({ reason, label: reason, ignored }) as SkippedFile;
  const groups = skippedByReason([
    file("video_file"),
    file("video_file"),
    file("drm_protected"),
    file("drm_protected", true),
  ]);
  expect(groups.map((group) => group.items.length)).toEqual([2, 1]);
});

test("job lines show progress while running", () => {
  const job = {
    state: "running",
    total: 10,
    done: 4,
    current: "Album",
    message: "",
    title: "Re-analyze audio",
  } as ToolJob;
  expect(jobLine(job)).toBe("4 of 10 · Album");
  expect(jobLine({ ...job, state: "completed", message: "Done." })).toBe(
    "Done.",
  );
});
