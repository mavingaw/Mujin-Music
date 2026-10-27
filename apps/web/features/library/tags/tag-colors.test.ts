import { describe, expect, test } from "bun:test";
import { TAG_COLORS } from "@parson/music-sdk";
import { fallbackTagColor, sortTags, tagColor, tagTone } from "./tag-colors";

describe("tag colours", () => {
  test("a chosen colour wins and every colour has a tone", () => {
    expect(tagColor({ name: "Late night", color: "violet" })).toBe("violet");
    for (const color of TAG_COLORS) {
      expect(tagTone({ name: "x", color }).dot).toStartWith("bg-");
    }
  });

  test("uncoloured tags get a stable colour from their name", () => {
    expect(fallbackTagColor("Late Night")).toBe(fallbackTagColor("late night"));
    expect(TAG_COLORS).toContain(tagColor({ name: "Sunday", color: null }));
  });

  test("tags sort by name, size or recency", () => {
    const tags = [
      { name: "b", item_count: 1, updated_at: "2026-10-02 10:00:00" },
      { name: "A", item_count: 5, updated_at: "2026-10-01 10:00:00" },
      { name: "c", item_count: 5, updated_at: "2026-10-03 10:00:00" },
    ];
    expect(sortTags(tags, "name").map((tag) => tag.name)).toEqual([
      "A",
      "b",
      "c",
    ]);
    expect(sortTags(tags, "items").map((tag) => tag.name)).toEqual([
      "A",
      "c",
      "b",
    ]);
    expect(sortTags(tags, "updated").map((tag) => tag.name)).toEqual([
      "c",
      "b",
      "A",
    ]);
  });
});
