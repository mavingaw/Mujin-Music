/**
 * Room the app header leaves for Parson Desktop's own window controls
 * (minimise, maximise, close) on Windows and Linux, where the window has no
 * system title bar. Without it the search field runs under the controls in
 * a narrow window.
 */
export const WINDOW_CONTROL_WIDTH_PX = 48;
export const WINDOW_CONTROL_COUNT = 3;
export const WINDOW_CONTROLS_WIDTH_PX =
  WINDOW_CONTROL_WIDTH_PX * WINDOW_CONTROL_COUNT;
/** Gap kept between the search field and the controls. */
export const WINDOW_CONTROLS_GAP_PX = 8;

/** The header's right padding (px) for a desktop platform, or 0 for none. */
export function windowControlsReserve(
  platform: "windows" | "linux" | "macos" | null,
): number {
  return platform === "windows" || platform === "linux"
    ? WINDOW_CONTROLS_WIDTH_PX + WINDOW_CONTROLS_GAP_PX
    : 0;
}

// The header's geometry below `md` (see app-shell.tsx): `px-4` padding, the
// logo button in the first 48 px, the search field after it.
const HEADER_PADDING_PX = 16;
const LOGO_SPACE_PX = 48;

/**
 * Where the search field sits in a narrow window (below `md`), and whether
 * the window controls would cover it.
 */
export function narrowTitlebar(viewportWidth: number, reservePx: number) {
  const contentRight = viewportWidth - HEADER_PADDING_PX - reservePx;
  const searchLeft = HEADER_PADDING_PX + LOGO_SPACE_PX;
  const searchRight = reservePx ? contentRight : contentRight - LOGO_SPACE_PX;
  const controlsLeft = reservePx
    ? viewportWidth - HEADER_PADDING_PX - WINDOW_CONTROLS_WIDTH_PX
    : viewportWidth;
  return {
    searchLeft,
    searchRight,
    searchWidth: Math.max(0, searchRight - searchLeft),
    controlsLeft,
    overlaps: searchRight > controlsLeft,
  };
}
