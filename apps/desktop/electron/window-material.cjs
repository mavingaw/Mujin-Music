"use strict";

/**
 * Native window materials for the Frosted Glass theme: Windows 11's Mica
 * and Acrylic (Electron's `backgroundMaterial`, Windows 11 22H2 and later)
 * and macOS vibrancy. Elsewhere nothing is supported and the web app
 * paints its own blurred backdrop instead.
 */

const WINDOWS_MATERIAL_BUILD = 22621;

/** The materials this platform can show behind a transparent page. */
function windowMaterialSupport(platform, release) {
  if (platform === "win32") {
    const build = Number(String(release ?? "").split(".")[2]);
    return Number.isFinite(build) && build >= WINDOWS_MATERIAL_BUILD
      ? ["mica", "acrylic"]
      : [];
  }
  if (platform === "darwin") return ["vibrancy"];
  return [];
}

/**
 * Applies a requested material (or none) to a window. Returns what was
 * applied: the material, or null when none was asked for or the platform
 * cannot show it. The window's opaque background colour comes back with
 * `null`, so turning the theme off never leaves a see-through window.
 */
function applyWindowMaterial(window, requested, support, opaque = "#000000") {
  const material = support.includes(requested) ? requested : null;
  try {
    if (support.includes("mica") || support.includes("acrylic")) {
      window.setBackgroundMaterial?.(material ?? "none");
    }
    if (support.includes("vibrancy")) {
      window.setVibrancy?.(material === "vibrancy" ? "under-window" : null);
    }
    window.setBackgroundColor?.(material ? "#00000000" : opaque);
  } catch {
    window.setBackgroundColor?.(opaque);
    return null;
  }
  return material;
}

module.exports = { applyWindowMaterial, windowMaterialSupport };
