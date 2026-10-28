import { describe, expect, test } from "bun:test";
import { DEFAULT_APPEARANCE } from "@parson/music-sdk";
import { THEME_BOOT_SCRIPT } from "./theme-boot";
import {
  buildThemeCache,
  readThemeCache,
  THEME_CACHE_KEY,
  THEME_STYLE_ID,
} from "./theme-cache";

function runBoot(cache: string | null, prefersDark: boolean, path = "/") {
  const attributes = new Map<string, string>();
  const appended: { id: string; textContent: string }[] = [];
  const meta = {
    content: "#0A0A0B",
    setAttribute(_: string, v: string) {
      this.content = v;
    },
  };
  const document = {
    createElement: () => ({ id: "", textContent: "" }),
    head: {
      appendChild: (node: { id: string; textContent: string }) =>
        appended.push(node),
    },
    documentElement: {
      setAttribute: (name: string, value: string) =>
        attributes.set(name, value),
      removeAttribute: (name: string) => attributes.delete(name),
    },
    querySelector: () => meta,
  };
  const window = { matchMedia: () => ({ matches: prefersDark }) };
  const localStorage = {
    getItem: (key: string) => (key === THEME_CACHE_KEY ? cache : null),
  };
  new Function(
    "location",
    "localStorage",
    "window",
    "document",
    THEME_BOOT_SCRIPT,
  )({ pathname: path }, localStorage, window, document);
  return { attributes, appended, meta };
}

describe("theme cache", () => {
  const followSystem = buildThemeCache(
    { ...DEFAULT_APPEARANCE, color_scheme: "system" },
    4,
    {},
    "7",
  );

  test("keeps both forms of a theme that follows the system", () => {
    expect(followSystem.dark.attrs["data-mode"]).toBe("dark");
    expect(followSystem.light.attrs["data-mode"]).toBe("light");
    expect(followSystem.light.css).toContain("--color-mujin-base:#f2eee6");
    const read = readThemeCache(JSON.stringify(followSystem));
    expect(read?.revision).toBe(4);
    expect(read?.userId).toBe("7");
    expect(readThemeCache("{oops")).toBeNull();
    expect(readThemeCache(JSON.stringify({ v: 2 }))).toBeNull();
  });

  test("the boot script paints the right form before the app starts", () => {
    const light = runBoot(JSON.stringify(followSystem), false);
    expect(light.attributes.get("data-mode")).toBe("light");
    expect(light.appended[0]?.id).toBe("mj-theme");
    expect(light.appended[0]?.textContent).toContain("color-scheme:light");
    expect(light.meta.content).toBe("#f2eee6");
    const dark = runBoot(JSON.stringify(followSystem), true);
    expect(dark.attributes.get("data-mode")).toBe("dark");
  });

  test("the boot script leaves the TV display and an empty cache alone", () => {
    expect(
      runBoot(JSON.stringify(followSystem), true, "/display").appended,
    ).toHaveLength(0);
    expect(runBoot(null, true).appended).toHaveLength(0);
    expect(runBoot("not json", true).appended).toHaveLength(0);
  });

  test("the boot script reads the same cache key and style id as the app", () => {
    expect(THEME_BOOT_SCRIPT).toContain(JSON.stringify(THEME_CACHE_KEY));
    expect(THEME_BOOT_SCRIPT).toContain(JSON.stringify(THEME_STYLE_ID));
  });
});
