import { describe, expect, test } from "bun:test";
import type { HistoryEntry, ListenLaterItem } from "@parson/music-sdk";
import {
  TAG_HEX,
  historySections,
  listenLaterRows,
  tagHex,
} from "./browse-collections";

const entry = (id: number, at: Date): HistoryEntry => ({
  id,
  songId: `s${id}`,
  listenedAt: at.toISOString(),
  title: `T${id}`,
  artistName: "A",
  durationSeconds: 120,
  available: true,
});

describe("browse collections", () => {
  test("tags get their colour or a stable fallback", () => {
    expect(tagHex({ name: "x", color: "sky" })).toBe(TAG_HEX.sky);
    expect(tagHex({ name: "Late Night" })).toBe(tagHex({ name: "late night" }));
  });

  test("history groups into day sections", () => {
    const now = new Date(2026, 9, 27, 21);
    const sections = historySections(
      [
        entry(3, new Date(2026, 9, 27, 20)),
        entry(2, new Date(2026, 9, 27, 8)),
        entry(1, new Date(2026, 9, 26, 22)),
      ],
      now,
    );
    expect(sections.map((section) => section.title)).toEqual([
      "Today",
      "Yesterday",
    ]);
    expect(sections[0]?.subtitle).toBe("2 plays · 4 min");
  });

  test("listen later rows describe each kind and skip missing ones", () => {
    const items: ListenLaterItem[] = [
      {
        entityType: "artist",
        entityId: "ar",
        addedAt: "",
        artist: {
          id: "ar",
          name: "Nina",
          artworkPath: "a.jpg",
          albumCount: 2,
          appearanceCount: 0,
          songCount: 20,
        },
      },
      { entityType: "album", entityId: "gone", addedAt: "" },
    ];
    const rows = listenLaterRows(items);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      title: "Nina",
      subtitle: "Artist · 2 albums",
      round: true,
    });
  });
});
