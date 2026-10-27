import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  DEFAULT_APPEARANCE,
  FONT_CHOICES,
  resolveTheme,
} from "@parson/music-sdk";
import { FONT_SAMPLE_STACKS } from "./appearance-model";

const WEB_ROOT = join(import.meta.dir, "..", "..");
const css = readFileSync(join(WEB_ROOT, "app", "globals.css"), "utf8");

interface FontFace {
  family: string;
  sources: string[];
}

function fontFaces(stylesheet: string): FontFace[] {
  return [...stylesheet.matchAll(/@font-face\s*\{([^}]*)\}/g)].map(
    ([, body]) => ({
      family: /font-family:\s*"([^"]+)"/.exec(body!)?.[1] ?? "",
      sources: [...body!.matchAll(/url\("([^"]+)"\)/g)].map(([, url]) => url!),
    }),
  );
}

/** The first family of a CSS font stack, unquoted. */
function firstFamily(stack: string): string {
  return stack.split(",")[0]!.trim().replace(/^"|"$/g, "");
}

const GENERIC = new Set(["serif", "sans-serif", "monospace", "system-ui"]);

describe("bundled fonts", () => {
  const faces = fontFaces(css);

  test("every @font-face points at a woff2 that ships with the app", () => {
    expect(faces.length).toBeGreaterThan(0);
    for (const face of faces) {
      expect(face.sources.length).toBeGreaterThan(0);
      for (const source of face.sources) {
        const file = join(WEB_ROOT, "public", source);
        expect(existsSync(file)).toBe(true);
        expect(readFileSync(file).subarray(0, 4).toString("latin1")).toBe(
          "wOF2",
        );
      }
      // Each bundled font folder carries its licence.
      const folder = face.sources[0]!.split("/").slice(0, -1).join("/");
      expect(existsSync(join(WEB_ROOT, "public", folder, "OFL.txt"))).toBe(
        true,
      );
    }
  });

  test("every font in the picker has an @font-face", () => {
    const families = new Set(faces.map((face) => face.family));
    for (const font of FONT_CHOICES) {
      const theme = resolveTheme(
        { ...DEFAULT_APPEARANCE, font },
        { systemDark: true },
      );
      const wanted = [
        firstFamily(FONT_SAMPLE_STACKS[font]),
        firstFamily(theme.type.display),
        firstFamily(theme.type.body),
      ];
      for (const family of wanted) {
        expect(GENERIC.has(family)).toBe(false);
        expect({ font, family, bundled: families.has(family) }).toEqual({
          font,
          family,
          bundled: true,
        });
      }
    }
  });
});
