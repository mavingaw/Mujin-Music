import { expect, test } from "bun:test";
import type { ScrobbleLogEntry } from "@parson/music-sdk";
import {
  activityLabel,
  importProgressLabel,
  listenTime,
} from "./scrobbling-model";

const entry = (overrides: Partial<ScrobbleLogEntry>): ScrobbleLogEntry => ({
  id: 1,
  service: "lastfm",
  songId: "s",
  artist: "A",
  track: "T",
  album: null,
  listenedAt: 0,
  status: "sent",
  attempts: 1,
  lastError: null,
  nextAttemptAt: 0,
  sentAt: 1,
  ...overrides,
});

test("activity chips", () => {
  expect(activityLabel(entry({}))).toEqual({ tone: "ok", text: "Scrobbled" });
  expect(activityLabel(entry({ status: "pending", attempts: 0 })).text).toBe(
    "Sending",
  );
  expect(activityLabel(entry({ status: "pending", attempts: 2 })).text).toBe(
    "Waiting to retry",
  );
  expect(activityLabel(entry({ status: "ignored" })).tone).toBe("idle");
  expect(activityLabel(entry({ status: "failed" })).tone).toBe("error");
});

test("import progress reads like a sentence", () => {
  const base = {
    error: null,
    startedAt: 0,
    finishedAt: null,
  };
  expect(
    importProgressLabel({
      ...base,
      status: "running",
      fetched: 0,
      matched: 0,
      imported: 0,
    }),
  ).toBe("Importing…");
  expect(
    importProgressLabel({
      ...base,
      status: "done",
      fetched: 2400,
      matched: 1900,
      imported: 1800,
    }),
  ).toBe(
    "Imported 2,400 listens: 1,800 listens added, 100 already here, 500 not in your library",
  );
  expect(
    importProgressLabel({
      ...base,
      status: "failed",
      fetched: 0,
      matched: 0,
      imported: 0,
      error: "Rate limited",
    }),
  ).toBe("Import stopped: Rate limited");
});

test("listen times are relative to today", () => {
  const now = new Date(2026, 9, 26, 15, 0);
  const at = (date: Date) => Math.floor(date.getTime() / 1000);
  expect(listenTime(at(new Date(2026, 9, 26, 14, 5)), now)).toBe("14:05");
  expect(listenTime(at(new Date(2026, 9, 25, 22, 10)), now)).toBe(
    "Yesterday 22:10",
  );
  expect(listenTime(at(new Date(2026, 2, 3, 9, 0)), now)).toBe("3 Mar");
});
