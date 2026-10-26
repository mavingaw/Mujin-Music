import { describe, expect, test } from "bun:test";
import type { FocusAlbum, FocusGroup, FocusSummary } from "../domain/types";
import {
  describeFocus,
  discographySections,
  qualityBadge,
  focusQueryString,
  focusResultLabel,
  focusRoleLabel,
  focusSelectionCount,
  focusSelectionFromParams,
  focusSelectionToParams,
  removeFocusChip,
  splitFocusGroups,
  toggleFocusValue,
} from "./focus";

describe("focus selection", () => {
  test("reads only facet params and keeps each value once", () => {
    const selection = focusSelectionFromParams(
      new URLSearchParams(
        "id=beethoven&tab=discography&conductor=karajan&ensemble=po&ensemble=po&bit_depth=24&bogus=1&period=",
      ),
    );
    expect(selection).toEqual({
      conductor: ["karajan"],
      ensemble: ["po"],
      bit_depth: ["24"],
    });
    expect(focusSelectionCount(selection)).toBe(3);
  });

  test("writes back in group order and keeps other params", () => {
    const params = focusSelectionToParams(
      { bit_depth: ["24"], conductor: ["karajan"], composer: ["beethoven"] },
      "id=x&tab=discography&conductor=old",
    );
    expect(params.toString()).toBe(
      "id=x&tab=discography&conductor=karajan&composer=beethoven&bit_depth=24",
    );
    expect(focusQueryString({ quality: ["hires"], role: ["composer"] })).toBe(
      "role=composer&quality=hires",
    );
  });

  test("toggles values and removes empty groups", () => {
    let selection = toggleFocusValue({}, "ensemble", "po");
    selection = toggleFocusValue(selection, "ensemble", "bp");
    expect(selection).toEqual({ ensemble: ["po", "bp"] });
    selection = toggleFocusValue(selection, "ensemble", "po");
    selection = toggleFocusValue(selection, "ensemble", "bp");
    expect(selection).toEqual({});
  });

  test("removes a chip by its group=value key", () => {
    const selection = { conductor: ["mb:abc=1"], quality: ["hires"] };
    expect(removeFocusChip(selection, "conductor=mb:abc=1")).toEqual({
      quality: ["hires"],
    });
    expect(removeFocusChip(selection, "nonsense=1")).toBe(selection);
    expect(removeFocusChip(selection, "quality=lossy")).toBe(selection);
  });
});

describe("focus presentation", () => {
  const group = (key: FocusGroup["key"], selected = false): FocusGroup => ({
    key,
    label: key,
    values: [{ key: "x", label: "X", count: 1, selected }],
  });

  test("shows a short list whole and keeps chosen groups in front of More", () => {
    const few = splitFocusGroups([group("role"), group("source")]);
    expect(few.more).toEqual([]);
    const { primary, more } = splitFocusGroups([
      group("role"),
      group("performer"),
      group("conductor"),
      group("ensemble"),
      group("soloist"),
      group("instrument", true),
      group("composer"),
      group("form"),
      group("period"),
      group("bit_depth"),
      group("source"),
    ]);
    expect(primary.map((entry) => entry.key)).toEqual([
      "role",
      "performer",
      "conductor",
      "ensemble",
      "soloist",
      "instrument",
      "composer",
      "bit_depth",
    ]);
    expect(more.map((entry) => entry.key)).toEqual([
      "form",
      "period",
      "source",
    ]);
  });

  test("summarises results and choices", () => {
    const summary: FocusSummary = {
      total: 12,
      matched: 3,
      groups: [],
      selected: [
        {
          key: "conductor=k",
          label: "Herbert von Karajan",
          count: 3,
          selected: true,
        },
        { key: "bit_depth=24", label: "24-bit", count: 3, selected: true },
      ],
    };
    expect(focusResultLabel(summary)).toBe("3 of 12 albums");
    expect(focusResultLabel({ ...summary, matched: 12 })).toBe("12 albums");
    expect(
      focusResultLabel(
        { ...summary, total: 1, matched: 1 },
        { one: "recording", other: "recordings" },
      ),
    ).toBe("1 recording");
    expect(describeFocus(summary)).toBe("Herbert von Karajan · 24-bit");
    expect(describeFocus(null)).toBe("");
    expect(focusRoleLabel("main")).toBe("Main artist");
    expect(focusRoleLabel("mixer")).toBe("Mixing");
    expect(focusRoleLabel("orchestrator")).toBe("Orchestrator");
  });
});

describe("discography sections", () => {
  const album = (
    id: string,
    roles: string[],
    primary_type = "Album",
  ): FocusAlbum =>
    ({
      album: { id, name: id, primary_type },
      artist: { id: "a", name: "A" },
      year: 1960,
      roles,
      conductors: [],
      ensembles: [],
      format: "FLAC 24-bit / 96 kHz",
      quality: "hires",
      sources: ["local"],
    }) as unknown as FocusAlbum;

  test("puts each album in its most important section", () => {
    const sections = discographySections([
      album("composed", ["composer"]),
      album("single", ["main"], "Single"),
      album("own", ["main", "composer"]),
      album("conducted", ["conductor", "composer"]),
      album("produced", ["producer"]),
      album("guest", ["featured"]),
    ]);
    expect(
      sections.map((section) => [
        section.title,
        section.albums.map((entry) => entry.album.id),
      ]),
    ).toEqual([
      ["Albums", ["own"]],
      ["Singles & EPs", ["single"]],
      ["As conductor", ["conducted"]],
      ["Appears on", ["guest"]],
      ["Recordings of their music", ["composed"]],
      ["Production & engineering", ["produced"]],
    ]);
  });

  test("badges read like a spec sheet", () => {
    expect(
      qualityBadge({ format: "FLAC 24-bit / 96 kHz", quality: "hires" }),
    ).toBe("24/96");
    expect(
      qualityBadge({ format: "WAV 16-bit / 22.05 kHz", quality: "lossless" }),
    ).toBe("16/22.05");
    expect(qualityBadge({ format: "MP3 320 kbps", quality: "lossy" })).toBe(
      "MP3",
    );
    expect(qualityBadge({ format: null, quality: null })).toBeNull();
  });
});
