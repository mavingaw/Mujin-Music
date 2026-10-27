import {
  type ResolvedTheme,
  themeAttributes,
  themeCss,
} from "@parson/music-sdk";
import { THEME_STYLE_ID } from "./theme-cache";

/**
 * Writes a resolved theme into the page: one style element with every
 * variable, the <html> attributes the stylesheet keys on, and the browser
 * chrome colour. Calling it again replaces both, so a preview is instant.
 */
export function applyThemeToDocument(
  theme: ResolvedTheme,
  doc: Document = document,
): void {
  let style = doc.getElementById(THEME_STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = doc.createElement("style");
    style.id = THEME_STYLE_ID;
    doc.head.appendChild(style);
  }
  const css = themeCss(theme);
  if (style.textContent !== css) style.textContent = css;
  const root = doc.documentElement;
  for (const [name, value] of Object.entries(themeAttributes(theme))) {
    if (value == null) root.removeAttribute(name);
    else if (root.getAttribute(name) !== value) root.setAttribute(name, value);
  }
  root.classList.toggle("dark", theme.mode === "dark");
  const meta = doc.querySelector('meta[name="theme-color"]');
  meta?.setAttribute("content", theme.colors.canvas);
}
