import { expect, test } from "bun:test";
import {
  chipTone,
  facetChipAccessibilityLabel,
  focusEntityForSection,
  smartPlaylistName,
} from "./library-focus";

test("only catalog browsers have a Focus", () => {
  expect(focusEntityForSection("albums")).toBe("albums");
  expect(focusEntityForSection("artists")).toBe("artists");
  expect(focusEntityForSection("downloads")).toBeNull();
  expect(focusEntityForSection("liked")).toBeNull();
});

test("smart playlists are named after their focus", () => {
  expect(smartPlaylistName({ entity: "albums", filters: [] })).toBe(
    "Smart playlist",
  );
  expect(
    smartPlaylistName({
      entity: "albums",
      filters: [
        { facet: "quality", value: "hires" },
        { facet: "played", exclude: true },
      ],
    }),
  ).toBe("Hi-Res · Unplayed");
  const long = smartPlaylistName({
    entity: "songs",
    filters: [{ facet: "genre", value: "x".repeat(300) }],
  });
  expect(long.length).toBe(200);
});

test("facet chips describe their state for screen readers", () => {
  expect(chipTone(null)).toBe("idle");
  expect(facetChipAccessibilityLabel("Jazz", 2, null)).toBe(
    "Jazz, 2 items. Double tap to focus.",
  );
  expect(facetChipAccessibilityLabel("MP3", 1, "exclude")).toBe(
    "MP3, excluded. Double tap to clear.",
  );
});
