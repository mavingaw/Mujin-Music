import { expect, test } from "bun:test";

import {
  displayKeyStorageName,
  displayMetrics,
  visibleLyricRange,
} from "./display";

test("display keys are stored per zone under safe names", () => {
  expect(displayKeyStorageName(null)).toBe("mujin.display.follow");
  expect(displayKeyStorageName("client:abc-123")).toBe(
    "mujin.display.client_abc-123",
  );
  expect(displayKeyStorageName("sonos:RINCON/1 2")).toBe(
    "mujin.display.sonos_RINCON_1_2",
  );
});

test("a landscape tablet gets large art and type, a phone stays readable", () => {
  const tablet = displayMetrics(1180, 820);
  expect(tablet.landscape).toBeTrue();
  expect(tablet.cover).toBe(279);
  expect(tablet.title).toBeGreaterThan(60);
  const phone = displayMetrics(390, 844);
  expect(phone.landscape).toBeFalse();
  expect(phone.cover).toBe(117);
  expect(phone.lyric).toBeGreaterThanOrEqual(24);
  expect(phone.coverInLyrics).toBeLessThan(phone.cover);
});

test("only lines near the sung one are drawn", () => {
  expect(visibleLyricRange(0, -1)).toEqual({ start: 0, end: 0 });
  expect(visibleLyricRange(40, -1)).toEqual({ start: 0, end: 5 });
  expect(visibleLyricRange(40, 20)).toEqual({ start: 18, end: 25 });
  expect(visibleLyricRange(5, 4)).toEqual({ start: 2, end: 5 });
});
