import type {
  AlbumAbout,
  ArtistAbout,
  EditorialSource,
  EditorialText,
  ReviewScore,
  TextSource,
} from "../domain/types";

const LANGUAGE_NAMES: Record<string, string> = {
  en: "English",
  de: "German",
  fr: "French",
  es: "Spanish",
  it: "Italian",
  nl: "Dutch",
  sv: "Swedish",
  da: "Danish",
  pt: "Portuguese",
  ja: "Japanese",
  pl: "Polish",
};

/** Native names, shown next to the English one in the source list. */
const NATIVE_NAMES: Record<string, string> = {
  en: "English",
  de: "Deutsch",
  fr: "Français",
  es: "Español",
  it: "Italiano",
  nl: "Nederlands",
  sv: "Svenska",
  da: "Dansk",
  pt: "Português",
  ja: "日本語",
  pl: "Polski",
};

/** The language of a `wikipedia:<language>` source id. */
export function sourceLanguage(id: string) {
  return id.startsWith("wikipedia:") ? id.slice("wikipedia:".length) : null;
}

/** "German Wikipedia" for `wikipedia:de`. */
export function editorialSourceName(id: string) {
  const language = sourceLanguage(id);
  if (!language) return id;
  return `${LANGUAGE_NAMES[language] ?? language} Wikipedia`;
}

/** "Deutsch" for `wikipedia:de`; null when it is the same as the English name. */
export function editorialSourceNativeName(id: string) {
  const language = sourceLanguage(id);
  if (!language) return null;
  const native = NATIVE_NAMES[language];
  return native && native !== LANGUAGE_NAMES[language] ? native : null;
}

/** A short label for a source switcher pill: "English", "Deutsch". */
export function editorialSourceShortName(id: string) {
  const language = sourceLanguage(id);
  if (!language) return id;
  return NATIVE_NAMES[language] ?? LANGUAGE_NAMES[language] ?? language;
}

/**
 * The text sources of an album or artist page, in priority order. Older
 * servers send only the main texts; they become one source.
 */
export function pageTextSources(
  about:
    | Pick<AlbumAbout, "background" | "review" | "text_sources">
    | Pick<ArtistAbout, "biography" | "text_sources">
    | null
    | undefined,
): TextSource[] {
  if (!about) return [];
  if (about.text_sources?.length) return about.text_sources;
  const texts = [
    "background" in about ? about.background : null,
    "review" in about ? about.review : null,
    "biography" in about ? about.biography : null,
  ].filter((text): text is EditorialText => Boolean(text));
  const first = texts[0];
  if (!first) return [];
  const id = `${first.source}:${first.language}`;
  return [
    {
      id,
      name: editorialSourceName(id),
      language: first.language,
      preferred: true,
      background: "background" in about ? about.background : null,
      review: "review" in about ? about.review : null,
      biography: "biography" in about ? about.biography : null,
    },
  ];
}

/** The source a page shows: the one the listener picked when it still
 * has text, otherwise the preferred one. */
export function chooseTextSource(
  sources: readonly TextSource[],
  picked: string | null | undefined,
) {
  return (
    sources.find((source) => source.id === picked) ??
    sources.find((source) => source.preferred) ??
    sources[0] ??
    null
  );
}

/** Moves a source one place up (-1) or down (+1) in the priority list. */
export function moveEditorialSource(
  sources: readonly EditorialSource[],
  id: string,
  delta: -1 | 1,
): EditorialSource[] {
  const list = [...sources];
  const index = list.findIndex((source) => source.id === id);
  const target = index + delta;
  if (index < 0 || target < 0 || target >= list.length) return list;
  const [moved] = list.splice(index, 1);
  if (moved) list.splice(target, 0, moved);
  return list;
}

/** Turns a source on or off. The last source turned on stays on: a page
 * always has somewhere to get its text from. */
export function toggleEditorialSource(
  sources: readonly EditorialSource[],
  id: string,
): EditorialSource[] {
  const enabled = sources.filter((source) => source.enabled);
  return sources.map((source) => {
    if (source.id !== id) return source;
    if (source.enabled && enabled.length === 1) return source;
    return { ...source, enabled: !source.enabled };
  });
}

/** A rating as five stars (full, half and empty), when it is a fraction. */
export function reviewStars(score: Pick<ReviewScore, "score" | "max">) {
  if (score.score === null || score.max === null || score.max <= 0) {
    return null;
  }
  const outOfFive = Math.round((score.score / score.max) * 10) / 2;
  const full = Math.floor(outOfFive);
  const half = outOfFive - full >= 0.5 ? 1 : 0;
  return { full, half, empty: 5 - full - half, value: outOfFive };
}
