import { expect, test } from "bun:test";
import type { DuplicateGroup, SkippedFile, ToolJob } from "@parson/music-sdk";
import {
  barSegments,
  duplicateEvidenceLabel,
  finishedJobs,
  formatBytes,
  formatPlayingTime,
  formatTrackTime,
  jobProgressLabel,
  losslessShare,
  moveItem,
  parseServerTime,
  parseToolView,
  skippedSections,
} from "./tools-model";

const skipped = (
  path: string,
  reason: SkippedFile["reason"],
  ignored = false,
): SkippedFile => ({
  path,
  file_name: path.split("/").pop() ?? path,
  folder: "/m",
  root_path: "/m",
  reason,
  label: reason,
  detail: "",
  extension: "",
  size_bytes: 1,
  ignored,
  imported: false,
  fix: { summary: "" },
  first_seen_at: "",
});

const job = (overrides: Partial<ToolJob>): ToolJob => ({
  id: 1,
  kind: "reanalyze",
  title: "Re-analyze audio",
  scope: "Whole library",
  state: "running",
  total: 0,
  done: 0,
  failed: 0,
  current: "",
  message: "",
  started_at_ms: 0,
  ...overrides,
});

test("views fall back to the overview", () => {
  expect(parseToolView("duplicates")).toBe("duplicates");
  expect(parseToolView("nope")).toBe("overview");
  expect(parseToolView(null)).toBe("overview");
});

test("sizes and times read naturally", () => {
  expect(formatBytes(0)).toBe("0 B");
  expect(formatBytes(1_234_000_000)).toBe("1.2 GB");
  expect(formatBytes(512_000_000_000)).toBe("512 GB");
  expect(formatPlayingTime(3 * 86_400 + 4 * 3600)).toBe("3 d 4 hr");
  expect(formatPlayingTime(5 * 3600 + 12 * 60)).toBe("5 hr 12 min");
  expect(formatTrackTime(241.4)).toBe("4:01");
  expect(formatTrackTime(3725)).toBe("1:02:05");
});

test("quality bars keep tiny shares visible and count lossless tiers", () => {
  const tiers = [
    { key: "hi_res", label: "Hi-Res", tracks: 999 },
    { key: "lossy", label: "Lossy", tracks: 1 },
    { key: "unknown", label: "Unknown", tracks: 0 },
  ];
  const segments = barSegments(tiers);
  expect(segments).toHaveLength(2);
  expect(segments[1]?.width).toBe(0.75);
  expect(losslessShare(tiers)).toBe(100);
  expect(losslessShare([])).toBe(0);
});

test("skipped files group by reason and filter by text and ignore state", () => {
  const items = [
    skipped("/m/a.mkv", "video_file"),
    skipped("/m/b.mkv", "video_file", true),
    skipped("/m/c.m4p", "drm_protected"),
  ];
  const sections = skippedSections(items);
  expect(sections.map((section) => section.reason)).toEqual([
    "video_file",
    "drm_protected",
  ]);
  expect(sections[0]?.items).toHaveLength(1);
  expect(skippedSections(items, { showIgnored: true })[0]?.items).toHaveLength(
    2,
  );
  expect(skippedSections(items, { query: "C.M4P" })).toHaveLength(1);
  expect(skippedSections(items, { reason: "drm_protected" })).toHaveLength(1);
});

test("duplicate evidence prefers the fingerprint", () => {
  const group = {
    evidence: {
      tags: true,
      isrc: true,
      fingerprint_similarity: 0.973,
      duration_spread: 0.4,
    },
  } as DuplicateGroup;
  expect(duplicateEvidenceLabel(group)).toContain("97% fingerprint match");
  expect(
    duplicateEvidenceLabel({
      evidence: { ...group.evidence, fingerprint_similarity: null },
    } as DuplicateGroup),
  ).toBe("Same ISRC recording code");
});

test("jobs report progress and which ones just finished", () => {
  expect(jobProgressLabel(job({ total: 2000, done: 12 }))).toBe("12 of 2,000");
  expect(jobProgressLabel(job({ done: 40 }))).toBe("40 checked");
  expect(
    jobProgressLabel(job({ state: "completed", message: "All good." })),
  ).toBe("All good.");
  const finished = finishedJobs(
    [job({ id: 1 }), job({ id: 2, state: "completed" })],
    [job({ id: 1, state: "completed" }), job({ id: 2, state: "completed" })],
  );
  expect(finished.map((item) => item.id)).toEqual([1]);
});

test("album track order moves one item at a time", () => {
  expect(moveItem(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
  expect(moveItem(["a", "b"], 0, 5)).toEqual(["a", "b"]);
});

test("server timestamps are read as UTC", () => {
  expect(parseServerTime("2026-10-27 01:23:45")?.toISOString()).toBe(
    "2026-10-27T01:23:45.000Z",
  );
  expect(parseServerTime("nonsense")).toBeNull();
  expect(parseServerTime(null)).toBeNull();
});
