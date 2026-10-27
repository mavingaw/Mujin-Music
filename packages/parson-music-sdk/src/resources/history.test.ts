import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  exportHistoryCsv,
  formatListeningTime,
  getHistoryStats,
  getHistoryTimeline,
  groupHistoryByDay,
  historyDayLabel,
  historyParams,
  historyPeriod,
  historySourceLabel,
  historyZoneLabel,
  periodChange,
  type HistoryEntry,
} from "./history";

afterEach(() => {
  mock.restore();
});

const response = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

function entry(id: number, listenedAt: Date, seconds = 200): HistoryEntry {
  return {
    id,
    songId: `s${id}`,
    listenedAt: listenedAt.toISOString(),
    title: `Track ${id}`,
    artistName: "Artist",
    durationSeconds: seconds,
    available: true,
  };
}

test("filters become snake_case parameters", () => {
  expect(
    historyParams({
      from: "2026-10-01T00:00:00.000Z",
      zone: "client:abc",
      artistId: "ar",
      albumId: "al",
      songId: "s",
      q: "  blue ",
      tz: 60.4,
    }),
  ).toEqual({
    from: "2026-10-01T00:00:00.000Z",
    zone: "client:abc",
    artist_id: "ar",
    album_id: "al",
    song_id: "s",
    q: "blue",
    tz: 60,
  });
  expect(historyParams({ q: "   " })).toEqual({});
});

test("timeline, stats and export call their endpoints", async () => {
  const get = spyOn(api, "get").mockImplementation(async (path) => {
    if (String(path).endsWith("export.csv")) return response("a,b\r\n");
    if (String(path).endsWith("timeline")) {
      return response({ items: [], nextBeforeId: 7 });
    }
    return response({ totals: { plays: 3 } });
  });
  expect(
    await getHistoryTimeline({ zone: "none" }, { beforeId: 9, limit: 50 }),
  ).toEqual({ items: [], nextBeforeId: 7 });
  expect(get).toHaveBeenCalledWith("/users/me/history/timeline", {
    params: { zone: "none", before_id: 9, limit: 50 },
  });
  await getHistoryStats({ tz: 0 }, { bucket: "week", top: 5 });
  expect(get).toHaveBeenLastCalledWith("/users/me/history/stats", {
    params: { tz: 0, bucket: "week", top: 5 },
  });
  expect(await exportHistoryCsv({ source: "manual" })).toBe("a,b\r\n");
  expect(get).toHaveBeenLastCalledWith("/users/me/history/export.csv", {
    params: { source: "manual" },
    responseType: "text",
    timeout: 60_000,
  });
});

test("periods cover whole local days and pick a chart bucket", () => {
  const now = new Date(2026, 9, 27, 15, 30);
  const week = historyPeriod("week", now);
  expect(new Date(week.from!).getTime()).toBe(new Date(2026, 9, 21).getTime());
  expect(new Date(week.to!).getTime()).toBe(new Date(2026, 9, 28).getTime());
  expect(week.bucket).toBe("day");
  const today = historyPeriod("today", now);
  expect(new Date(today.from!).getTime()).toBe(new Date(2026, 9, 27).getTime());
  const year = historyPeriod("year", now);
  expect(new Date(year.from!).getTime()).toBe(new Date(2026, 0, 1).getTime());
  expect(year.bucket).toBe("month");
  const lastYear = historyPeriod("last-year", now);
  expect(new Date(lastYear.to!).getTime()).toBe(new Date(2026, 0, 1).getTime());
  expect(historyPeriod("all", now)).toEqual({ bucket: "month" });
});

test("entries group by local day in order", () => {
  const now = new Date(2026, 9, 27, 20, 0);
  const days = groupHistoryByDay(
    [
      entry(5, new Date(2026, 9, 27, 18, 0), 100),
      entry(4, new Date(2026, 9, 27, 9, 0), 50),
      entry(3, new Date(2026, 9, 26, 23, 59)),
      entry(2, new Date(2026, 9, 20, 12, 0)),
    ],
    now,
  );
  expect(days.map((day) => day.label)).toEqual([
    "Today",
    "Yesterday",
    historyDayLabel("2026-10-20", now),
  ]);
  expect(days[0]?.items.map((item) => item.id)).toEqual([5, 4]);
  expect(days[0]?.listenedSeconds).toBe(150);
  expect(historyDayLabel("2026-10-24", now)).toBe("Saturday");
  expect(historyDayLabel("2025-03-02", now)).toBe("2 March 2025");
});

test("listening time, change and labels read naturally", () => {
  expect(formatListeningTime(59)).toBe("1 min");
  expect(formatListeningTime(3600 * 3 + 1200)).toBe("3 h 20 min");
  expect(formatListeningTime(3600 * 2)).toBe("2 h");
  expect(formatListeningTime(3600 * 52)).toBe("2 days 4 h");
  expect(periodChange(150, 100)).toBe(0.5);
  expect(periodChange(10, 0)).toBeNull();
  expect(periodChange(10, null)).toBeNull();
  expect(historySourceLabel("generated")).toBe("Radio");
  expect(historySourceLabel("manual")).toBe("Your picks");
  expect(historySourceLabel(null)).toBe("Unknown");
  expect(historySourceLabel("playlist")).toBe("Playlist");
  expect(historyZoneLabel({ value: "none", label: null })).toBe("Not recorded");
  expect(historyZoneLabel({ value: "client:a", label: " Office " })).toBe(
    "Office",
  );
});
