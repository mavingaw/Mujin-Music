import { expect, test } from "bun:test";
import {
  WINDOW_CONTROLS_WIDTH_PX,
  narrowTitlebar,
  windowControlsReserve,
} from "./titlebar-layout";

test("the desktop window controls never cover the search field at 412 px", () => {
  const reserve = windowControlsReserve("windows");
  expect(reserve).toBeGreaterThanOrEqual(WINDOW_CONTROLS_WIDTH_PX);
  for (const width of [360, 412, 600, 767]) {
    const layout = narrowTitlebar(width, reserve);
    expect(layout.overlaps).toBe(false);
    expect(layout.searchWidth).toBeGreaterThan(100);
  }
  // Without the reserve (the old header) the controls covered the field.
  expect(narrowTitlebar(412, 0).searchRight).toBeGreaterThan(
    412 - 16 - WINDOW_CONTROLS_WIDTH_PX,
  );
  // macOS keeps its own traffic lights in a system title bar.
  expect(windowControlsReserve("macos")).toBe(0);
  expect(windowControlsReserve(null)).toBe(0);
  expect(windowControlsReserve("linux")).toBe(reserve);
});
