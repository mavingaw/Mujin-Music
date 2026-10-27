import { expect, test } from "bun:test";
import {
  EMPTY_PREFS,
  arrangeColumns,
  columnsPerPage,
  pageLayout,
  pageStarts,
  parseColumnPrefs,
  resumeStart,
  setValueOrder,
  toggleHidden,
  togglePinned,
} from "./focus-columns";

const keys = [
  "genre:",
  "year:",
  "credit:performer",
  "credit:conductor",
  "credit:composer",
  "format:",
  "sample_rate:",
  "tag:",
];
const self = (key: string) => key;

test("a page holds only whole columns", () => {
  // 1040px of panel: five 168px columns with 24px gaps fit (936px), six do not.
  expect(columnsPerPage(1040)).toBe(5);
  expect(columnsPerPage(1127)).toBe(5);
  expect(columnsPerPage(1128)).toBe(6);
  expect(columnsPerPage(120)).toBe(1);
  expect(columnsPerPage(0)).toBe(1);
});

test("paging moves by whole pages and the last page ends flush", () => {
  const first = pageLayout(0, 8, 5, 0);
  expect(first).toMatchObject({
    start: 0,
    end: 5,
    canPrevious: false,
    canNext: true,
    next: 3,
  });
  // The last page shows columns 4-8: never a half column, never blank slots.
  const last = pageLayout(0, 8, 5, first.next);
  expect(last).toMatchObject({
    start: 3,
    end: 8,
    canPrevious: true,
    canNext: false,
    previous: 0,
  });
  // A stale start beyond the end is pulled back.
  expect(pageLayout(0, 8, 5, 40).start).toBe(3);
  expect(pageLayout(0, 3, 5, 2)).toMatchObject({
    start: 0,
    end: 3,
    canNext: false,
  });
});

test("arrows step along the page grid", () => {
  expect(pageStarts(8, 5)).toEqual([0, 3]);
  expect(pageStarts(14, 4)).toEqual([0, 4, 8, 10]);
  expect(pageStarts(3, 5)).toEqual([0]);
  // A start between pages (columns changed under it) rejoins the grid.
  const between = pageLayout(0, 14, 4, 2);
  expect(between).toMatchObject({ start: 2, page: 0, next: 4, previous: 0 });
  expect(pageLayout(0, 14, 4, 9)).toMatchObject({
    page: 2,
    next: 10,
    previous: 8,
  });
  expect(pageLayout(0, 14, 4, 10)).toMatchObject({
    page: 3,
    canNext: false,
    previous: 8,
  });
});

test("pinned columns stay put while the rest page past them", () => {
  const prefs = togglePinned(togglePinned(EMPTY_PREFS, "tag:"), "format:");
  const { pinned, rest } = arrangeColumns(keys, self, prefs);
  expect(pinned).toEqual(["tag:", "format:"]);
  expect(rest[0]).toBe("genre:");
  const layout = pageLayout(pinned.length, rest.length, 5, 0);
  expect(layout).toMatchObject({ pinnedShown: 2, slots: 3, end: 3 });
  expect(pageLayout(pinned.length, rest.length, 5, layout.next).start).toBe(3);
  // Pins never take the only slot left for paging.
  expect(pageLayout(4, 4, 3, 0)).toMatchObject({ pinnedShown: 2, slots: 1 });
  expect(pageLayout(2, 0, 3, 0)).toMatchObject({ pinnedShown: 2, slots: 0 });
});

test("hidden columns leave the panel and hiding unpins", () => {
  let prefs = togglePinned(EMPTY_PREFS, "year:");
  prefs = toggleHidden(prefs, "year:");
  expect(prefs.pinned).toEqual([]);
  const arranged = arrangeColumns(keys, self, prefs);
  expect(arranged.hidden).toEqual(["year:"]);
  expect(arranged.rest).not.toContain("year:");
  prefs = toggleHidden(prefs, "year:");
  expect(arrangeColumns(keys, self, prefs).rest).toContain("year:");
  // Pinning a hidden column shows it again.
  prefs = togglePinned(toggleHidden(prefs, "tag:"), "tag:");
  expect(prefs.hidden).toEqual([]);
});

test("a click on the first page never pages away from Genres", () => {
  // Clicking a value can add or drop columns; the first page stays first.
  expect(resumeStart(0, "genre:", keys.slice(1))).toBe(0);
  // Further along, the column that led the page leads it again.
  const after = keys.filter((key) => key !== "credit:performer");
  expect(resumeStart(3, "credit:conductor", keys)).toBe(3);
  expect(resumeStart(3, "credit:conductor", after)).toBe(2);
  // A vanished leader keeps the old position (pageLayout clamps it).
  expect(resumeStart(3, "gone:", after)).toBe(3);
});

test("stored layouts are validated", () => {
  expect(parseColumnPrefs(null)).toEqual(EMPTY_PREFS);
  expect(parseColumnPrefs("{not json")).toEqual(EMPTY_PREFS);
  expect(parseColumnPrefs("[1,2]")).toEqual(EMPTY_PREFS);
  expect(
    parseColumnPrefs(
      JSON.stringify({
        pinned: ["tag:", 4],
        hidden: "x",
        order: { "genre:": "name", "tag:": "random" },
      }),
    ),
  ).toEqual({ pinned: ["tag:"], hidden: [], order: { "genre:": "name" } });
  const named = setValueOrder(EMPTY_PREFS, "genre:", "name");
  expect(named.order).toEqual({ "genre:": "name" });
  expect(setValueOrder(named, "genre:", "count").order).toEqual({});
});
