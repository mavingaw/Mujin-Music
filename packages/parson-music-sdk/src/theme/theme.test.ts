import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
import api, { ApiError } from "../core/http";
import {
  appearanceConflict,
  getAppearance,
  saveAppearance,
} from "../resources/appearance";
import {
  DEFAULT_APPEARANCE,
  HOME_ROWS,
  THEME_PRESETS,
  moveHomeRow,
  normalizeAppearance,
  placeHomeRow,
  visibleHomeRows,
} from "./appearance";
import { scoreArtworkColors, sourceColorFromPixels } from "./artwork-color";
import { argbFromHex, contrastRatio, hexFromArgb } from "./color";
import { themeAttributes, themeCss, themeVariables } from "./css";
import {
  failingChecks,
  materialRoles,
  resolveMode,
  resolveTheme,
} from "./engine";
import { argbFromHct, hctFromArgb } from "./hct";
import { ensureContrast, TonalPalette } from "./palette";

afterEach(() => mock.restore());

const ratio = (a: string, b: string) =>
  contrastRatio(argbFromHex(a)!, argbFromHex(b)!);

describe("HCT", () => {
  test("matches Material's reference values", () => {
    const blue = hctFromArgb(0xff0000ff);
    expect(blue.hue).toBeCloseTo(282.788, 2);
    expect(blue.chroma).toBeCloseTo(87.23, 1);
    expect(blue.tone).toBeCloseTo(32.302, 2);
    const red = hctFromArgb(0xffff0000);
    expect(red.hue).toBeCloseTo(27.408, 2);
    expect(red.chroma).toBeCloseTo(113.357, 1);
    expect(red.tone).toBeCloseTo(53.233, 2);
  });

  test("round-trips and lands on the requested tone", () => {
    const copper = argbFromHex("#d98b5f")!;
    const hct = hctFromArgb(copper);
    expect(hexFromArgb(argbFromHct(hct.hue, hct.chroma, hct.tone))).toBe(
      "#d98b5f",
    );
    for (const tone of [10, 25, 40, 60, 80, 95]) {
      const made = argbFromHct(hct.hue, 200, tone);
      expect(Math.abs(hctFromArgb(made).tone - tone)).toBeLessThan(0.6);
    }
  });

  test("tonal palettes run from black to white", () => {
    const palette = TonalPalette.fromArgb(0xff3366ff);
    expect(hexFromArgb(palette.tone(0))).toBe("#000000");
    expect(hexFromArgb(palette.tone(100))).toBe("#ffffff");
  });
});

describe("contrast", () => {
  test("ensureContrast moves a tone just far enough", () => {
    const canvas = argbFromHex("#f2eee6")!;
    const copper = argbFromHex("#d98b5f")!;
    expect(contrastRatio(copper, canvas)).toBeLessThan(4.5);
    const fixed = ensureContrast(copper, canvas, 4.5);
    expect(contrastRatio(fixed, canvas)).toBeGreaterThanOrEqual(4.5);
    // Same hue family, only darker.
    expect(
      Math.abs(hctFromArgb(fixed).hue - hctFromArgb(copper).hue),
    ).toBeLessThan(6);
    expect(ensureContrast(copper, argbFromHex("#0a0a0b")!, 4.5)).toBe(copper);
  });

  test("every preset, mode and accent passes WCAG AA for text", () => {
    const accents = [
      null,
      "#ffff00",
      "#00ffff",
      "#0000ff",
      "#ff00ff",
      "#777777",
      "#ffffff",
      "#000000",
      "#3d5afe",
      "#00c853",
    ];
    for (const preset of THEME_PRESETS) {
      for (const color_scheme of ["dark", "light"] as const) {
        for (const accent of accents) {
          const theme = resolveTheme(
            { ...DEFAULT_APPEARANCE, preset, color_scheme, accent },
            { systemDark: true },
          );
          const failing = failingChecks(theme);
          if (failing.length) {
            throw new Error(
              `${preset}/${color_scheme}/${accent}: ${failing
                .map((check) => `${check.name} ${check.ratio}`)
                .join(", ")}`,
            );
          }
          const c = theme.colors;
          expect(ratio(c.text, c.canvas)).toBeGreaterThanOrEqual(7);
          expect(ratio(c.textSecondary, c.surface2)).toBeGreaterThanOrEqual(
            4.5,
          );
          expect(ratio(c.textTertiary, c.canvas)).toBeGreaterThanOrEqual(4.5);
          expect(ratio(c.accent, c.canvas)).toBeGreaterThanOrEqual(4.5);
          expect(ratio(c.onAccent, c.accent)).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  test("High Contrast reaches AAA-level text and strong borders", () => {
    for (const color_scheme of ["dark", "light"] as const) {
      const theme = resolveTheme(
        { ...DEFAULT_APPEARANCE, preset: "high-contrast", color_scheme },
        { systemDark: true },
      );
      expect(ratio(theme.colors.text, theme.colors.canvas)).toBeGreaterThan(15);
      expect(
        ratio(theme.colors.borderStrong, theme.colors.canvas),
      ).toBeGreaterThanOrEqual(4.5);
      expect(theme.contrast).toBe("high");
    }
  });
});

describe("presets", () => {
  test("Mujin Dark is the brand palette, untouched", () => {
    const theme = resolveTheme(DEFAULT_APPEARANCE, { systemDark: true });
    expect(theme.mode).toBe("dark");
    expect(theme.colors.canvas).toBe("#0a0a0b");
    expect(theme.colors.accent).toBe("#d98b5f");
    expect(theme.colors.onAccent).toBe("#160d07");
    expect(theme.neutralRamp).toBeNull();
    expect(theme.accentRamp).toBeNull();
    expect(theme.accentSource).toBe("brand");
  });

  test("Match system follows prefers-color-scheme for the Mujin pair", () => {
    const system = { ...DEFAULT_APPEARANCE, color_scheme: "system" as const };
    expect(resolveMode(system, true)).toBe("dark");
    expect(resolveMode(system, false)).toBe("light");
    const light = resolveTheme(system, { systemDark: false });
    expect(light.colors.canvas).toBe("#f2eee6");
    expect(light.colors.text).toBe("#0a0a0b");
    expect(
      resolveMode({ preset: "oled-black", color_scheme: "light" }, false),
    ).toBe("dark");
  });

  test("OLED Black is true black", () => {
    const theme = resolveTheme(
      { ...DEFAULT_APPEARANCE, preset: "oled-black" },
      { systemDark: false },
    );
    expect(theme.colors.canvas).toBe("#000000");
    expect(theme.colors.deep).toBe("#000000");
  });

  test("Now Playing and album headers follow the preset's backdrop", () => {
    const backdrop = (
      preset: (typeof THEME_PRESETS)[number],
      extra: Partial<typeof DEFAULT_APPEARANCE> = {},
      environment: Partial<Parameters<typeof resolveTheme>[1]> = {},
    ) => {
      const theme = resolveTheme(
        { ...DEFAULT_APPEARANCE, preset, ...extra },
        { systemDark: true, ...environment },
      );
      expect(themeAttributes(theme)["data-backdrop"]).toBe(theme.backdrop);
      return theme.backdrop;
    };
    // OLED Black keeps pixels off unless colour from artwork is on.
    expect(backdrop("oled-black")).toBe("black");
    expect(backdrop("oled-black", { accent_from_artwork: true })).toBe(
      "artwork",
    );
    // Frosted Glass puts panels of glass over the blurred cover...
    expect(backdrop("frosted-glass")).toBe("glass");
    // ...and turns opaque when transparency is reduced.
    expect(backdrop("frosted-glass", {}, { reducedTransparency: true })).toBe(
      "artwork",
    );
    // High Contrast drops the art wash for the plain canvas.
    expect(backdrop("high-contrast")).toBe("solid");
    expect(backdrop("high-contrast", { color_scheme: "light" })).toBe("solid");
    expect(backdrop("mujin-dark")).toBe("artwork");
    expect(backdrop("material-you")).toBe("artwork");
  });

  test("Material You builds a tonal scheme from the album art", () => {
    const art = argbFromHex("#1e88e5")!;
    const theme = resolveTheme(
      {
        ...DEFAULT_APPEARANCE,
        preset: "material-you",
        accent_from_artwork: true,
        color_scheme: "dark",
      },
      { systemDark: true, artworkColor: art },
    );
    expect(theme.accentSource).toBe("artwork");
    const accentHue = hctFromArgb(argbFromHex(theme.colors.accent)!).hue;
    expect(Math.abs(accentHue - hctFromArgb(art).hue)).toBeLessThan(8);
    // Surfaces are tinted with the same hue, faintly.
    const surface = hctFromArgb(argbFromHex(theme.colors.surface2)!);
    expect(surface.chroma).toBeGreaterThan(1.5);
    expect(surface.chroma).toBeLessThan(10);
    expect(theme.shape.material).toBeTrue();
    expect(theme.shape.scale).toBeCloseTo(1.75, 2);
    expect(theme.type.body).toContain("Roboto Flex");
    // Tones follow Material 3: primary T80 on dark, T40 on light.
    const roles = materialRoles(art, "dark");
    expect(hctFromArgb(roles.primary).tone).toBeCloseTo(80, 0);
    expect(hctFromArgb(materialRoles(art, "light").primary).tone).toBeCloseTo(
      40,
      0,
    );
  });

  test("Android's wallpaper colour wins when following the system", () => {
    const theme = resolveTheme(
      { ...DEFAULT_APPEARANCE, preset: "material-you" },
      { systemDark: true, systemAccent: 0xff6750a4, artworkColor: 0xff00ff00 },
    );
    expect(theme.accentSource).toBe("system");
  });

  test("Frosted Glass turns solid when transparency is reduced", () => {
    const glass = { ...DEFAULT_APPEARANCE, preset: "frosted-glass" as const };
    expect(resolveTheme(glass, { systemDark: true }).glass.enabled).toBeTrue();
    expect(
      resolveTheme(glass, { systemDark: true, reducedTransparency: true }).glass
        .enabled,
    ).toBeFalse();
    expect(
      resolveTheme({ ...glass, transparency: "reduced" }, { systemDark: true })
        .glass.enabled,
    ).toBeFalse();
    expect(
      resolveTheme(glass, {
        systemDark: true,
        nativeMaterials: ["mica", "acrylic"],
      }).glass.native,
    ).toBe("mica");
    expect(
      resolveTheme(
        { ...glass, glass_material: "acrylic" },
        { systemDark: true, nativeMaterials: ["mica", "acrylic"] },
      ).glass.native,
    ).toBe("acrylic");
    // macOS offers vibrancy; Linux nothing, so the painted backdrop stays.
    expect(
      resolveTheme(glass, { systemDark: true, nativeMaterials: ["vibrancy"] })
        .glass.native,
    ).toBe("vibrancy");
    expect(resolveTheme(glass, { systemDark: true }).glass.native).toBeNull();
  });
});

describe("appearance", () => {
  test("normalises anything into a valid appearance", () => {
    expect(normalizeAppearance(null)).toEqual(DEFAULT_APPEARANCE);
    const odd = normalizeAppearance({
      preset: "winamp",
      accent: "#ABC",
      corner_radius: 400,
      text_scale: "1.2",
      density: "compact",
      home_rows: [{ id: "genres", visible: false }, { id: "x" }],
    });
    expect(odd.preset).toBe("mujin-dark");
    expect(odd.accent).toBe("#aabbcc");
    expect(odd.corner_radius).toBe(24);
    expect(odd.text_scale).toBe(1.2);
    // Rows missing from the saved list return after their default
    // predecessor; the saved row keeps its choice.
    expect(odd.home_rows[0]).toEqual({ id: "daily-picks", visible: true });
    expect(odd.home_rows.find((row) => row.id === "genres")).toEqual({
      id: "genres",
      visible: false,
    });
    expect(odd.home_rows).toHaveLength(HOME_ROWS.length);
    expect(visibleHomeRows(odd)).not.toContain("genres");
  });

  test("Home rows move and drop into place", () => {
    const rows = DEFAULT_APPEARANCE.home_rows;
    expect(moveHomeRow(rows, "genres", -1).map((row) => row.id)[6]).toBe(
      "genres",
    );
    expect(moveHomeRow(rows, "daily-picks", -1)).toEqual(rows);
    expect(placeHomeRow(rows, "live-radio", 0)[0]!.id).toBe("live-radio");
  });
});

describe("CSS", () => {
  test("every surface reads from variables", () => {
    const theme = resolveTheme(
      { ...DEFAULT_APPEARANCE, preset: "mujin-light", color_scheme: "light" },
      { systemDark: false },
    );
    const vars = themeVariables(theme);
    expect(vars["--color-mujin-base"]).toBe("#f2eee6");
    expect(vars["--color-white"]).toBe(theme.colors.text);
    expect(vars["--color-zinc-900"]).toBeDefined();
    expect(vars["--color-indigo-400"]).toBe(theme.colors.accent);
    expect(vars["--color-sky-400"]).toBeDefined();
    expect(vars["--background"]).toMatch(/^\d+ \d+% \d+%$/);
    const css = themeCss(theme);
    expect(css.startsWith(":root{")).toBeTrue();
    expect(css).toContain("color-scheme:light");
    const attributes = themeAttributes(
      resolveTheme(
        {
          ...DEFAULT_APPEARANCE,
          preset: "frosted-glass",
          density: "compact",
          sidebar: "compact",
        },
        { systemDark: true },
      ),
    );
    expect(attributes["data-glass"]).toBe("on");
    expect(attributes["data-density"]).toBe("compact");
    expect(attributes["data-sidebar"]).toBe("compact");
    expect(
      themeVariables(
        resolveTheme(
          { ...DEFAULT_APPEARANCE, density: "compact" },
          { systemDark: true },
        ),
      )["--spacing"],
    ).toBe("0.225rem");
  });
});

describe("artwork colour", () => {
  const image = (colors: [number, number, number, number][]) => {
    const data: number[] = [];
    for (const [r, g, b, count] of colors) {
      for (let index = 0; index < count; index += 1) data.push(r, g, b, 255);
    }
    return data;
  };

  test("a coloured field beats grey, and a grey cover falls back", () => {
    const blueCover = image([
      [20, 60, 200, 600],
      [128, 128, 128, 400],
    ]);
    const source = hctFromArgb(sourceColorFromPixels(blueCover));
    expect(source.hue).toBeGreaterThan(250);
    expect(source.hue).toBeLessThan(300);
    expect(
      hexFromArgb(sourceColorFromPixels(image([[120, 120, 120, 100]]))),
    ).toBe("#d98b5f");
    const two = scoreArtworkColors(
      image([
        [200, 40, 40, 400],
        [40, 160, 60, 300],
      ]),
      3,
    );
    expect(two.length).toBe(2);
  });
});

describe("API", () => {
  test("reads and saves the profile's appearance", async () => {
    const get = spyOn(api, "get").mockResolvedValue({
      data: {
        appearance: { preset: "oled-black" },
        revision: 3,
        updated_at: 9,
      },
      status: 200,
      headers: new Headers(),
    } as never);
    const loaded = await getAppearance();
    expect(get).toHaveBeenCalledWith("/users/me/appearance");
    expect(loaded.appearance.preset).toBe("oled-black");
    expect(loaded.revision).toBe(3);
    const put = spyOn(api, "put").mockResolvedValue({
      data: { appearance: loaded.appearance, revision: 4, updated_at: 10 },
      status: 200,
      headers: new Headers(),
    } as never);
    expect((await saveAppearance(loaded.appearance, 3)).revision).toBe(4);
    expect(put).toHaveBeenCalledWith("/users/me/appearance", {
      appearance: loaded.appearance,
      base_revision: 3,
    });
  });

  test("a conflict carries the newer appearance", () => {
    const error = new ApiError("conflict", {}, {
      data: {
        error: "appearance_conflict",
        current: { appearance: { preset: "material-you" }, revision: 5 },
      },
      status: 409,
      headers: new Headers(),
    } as never);
    expect(appearanceConflict(error)?.appearance.preset).toBe("material-you");
    expect(appearanceConflict(new Error("x"))).toBeNull();
  });
});

describe("cover swatch", () => {
  test("decodes the server's base64 pixels and scores them", async () => {
    const { bytesFromBase64, sourceColorFromSwatch } = await import("./swatch");
    expect([...bytesFromBase64("AAEC/w==")]).toEqual([0, 1, 2, 255]);
    const pixels: number[] = [];
    for (let index = 0; index < 576; index += 1) pixels.push(30, 80, 200, 255);
    const rgba = Buffer.from(pixels).toString("base64");
    const source = sourceColorFromSwatch({ width: 24, height: 24, rgba });
    expect(hctFromArgb(source).hue).toBeGreaterThan(250);
    expect(hexFromArgb(sourceColorFromSwatch(null))).toBe("#d98b5f");
  });
});
