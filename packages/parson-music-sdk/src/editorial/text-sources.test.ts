import { describe, expect, test } from "bun:test";
import type { EditorialText, TextSource } from "../domain/types";
import {
  chooseTextSource,
  editorialSourceName,
  editorialSourceNativeName,
  editorialSourceShortName,
  moveEditorialSource,
  pageTextSources,
  reviewStars,
  toggleEditorialSource,
} from "./text-sources";
import { albumTextPanels } from "./works";

const text = (
  kind: EditorialText["kind"],
  language: string,
  body: string,
): EditorialText => ({
  kind,
  source: "wikipedia",
  language,
  title: "Kind of Blue",
  body,
  url: `https://${language}.wikipedia.org/wiki/Kind_of_Blue`,
  license: "CC BY-SA 4.0",
  license_url: "https://creativecommons.org/licenses/by-sa/4.0/",
});

const sources: TextSource[] = [
  {
    id: "wikipedia:en",
    name: "English Wikipedia",
    language: "en",
    preferred: true,
    biography: null,
    background: text("background", "en", "Lead"),
    review: text("reception", "en", "Reception"),
  },
  {
    id: "wikipedia:de",
    name: "German Wikipedia",
    language: "de",
    preferred: false,
    biography: null,
    background: text("background", "de", "Einleitung"),
    review: null,
  },
];

describe("editorial text sources", () => {
  test("names sources in English and in their own language", () => {
    expect(editorialSourceName("wikipedia:de")).toBe("German Wikipedia");
    expect(editorialSourceNativeName("wikipedia:de")).toBe("Deutsch");
    expect(editorialSourceNativeName("wikipedia:en")).toBeNull();
    expect(editorialSourceShortName("wikipedia:fr")).toBe("Français");
  });

  test("older servers' main texts become one source", () => {
    const [only] = pageTextSources({
      background: text("background", "en", "Lead"),
      review: null,
    });
    expect(only.id).toBe("wikipedia:en");
    expect(only.preferred).toBeTrue();
    expect(pageTextSources({ biography: null })).toEqual([]);
  });

  test("a picked source wins while it still has text", () => {
    expect(chooseTextSource(sources, "wikipedia:de")?.id).toBe("wikipedia:de");
    expect(chooseTextSource(sources, "wikipedia:fr")?.id).toBe("wikipedia:en");
    expect(chooseTextSource([], null)).toBeNull();
  });

  test("album panels follow the chosen source", () => {
    const about = {
      background: sources[0].background,
      review: sources[0].review,
      work_texts: [],
      text_sources: sources,
    };
    expect(albumTextPanels(about).map((panel) => panel.text.body)).toEqual([
      "Reception",
      "Lead",
    ]);
    expect(
      albumTextPanels(about, "wikipedia:de").map((panel) => [
        panel.label,
        panel.text.body,
      ]),
    ).toEqual([["About", "Einleitung"]]);
  });

  test("sources move and toggle, keeping one on", () => {
    const list = [
      { id: "wikipedia:en", enabled: true },
      { id: "wikipedia:de", enabled: false },
      { id: "wikipedia:fr", enabled: false },
    ];
    expect(
      moveEditorialSource(list, "wikipedia:fr", -1).map((s) => s.id),
    ).toEqual(["wikipedia:en", "wikipedia:fr", "wikipedia:de"]);
    expect(moveEditorialSource(list, "wikipedia:en", -1)).toEqual(list);
    expect(toggleEditorialSource(list, "wikipedia:en")).toEqual(list);
    expect(toggleEditorialSource(list, "wikipedia:de")[1].enabled).toBeTrue();
  });

  test("ratings become stars out of five", () => {
    expect(reviewStars({ score: 4.5, max: 5 })).toEqual({
      full: 4,
      half: 1,
      empty: 0,
      value: 4.5,
    });
    expect(reviewStars({ score: 4, max: 4 })?.full).toBe(5);
    expect(reviewStars({ score: 7, max: 10 })).toEqual({
      full: 3,
      half: 1,
      empty: 1,
      value: 3.5,
    });
    expect(reviewStars({ score: null, max: null })).toBeNull();
  });
});
