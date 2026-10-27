import type {
  AboutCredit,
  AlbumAbout,
  AlbumField,
  AlbumSourcesPatch,
  EditedCredit,
  FactSource,
  ReleaseMatch,
  ResolvedFact,
} from "../domain/types";

/** Fields in the order the editor and the facts panel list them. */
export const ALBUM_FIELDS: AlbumField[] = [
  "original_date",
  "release_date",
  "label",
  "catalog_number",
  "country",
  "barcode",
  "cover",
  "performers",
  "credits",
];

export const ALBUM_FIELD_LABELS: Record<AlbumField, string> = {
  original_date: "Original release",
  release_date: "This edition",
  label: "Label",
  catalog_number: "Catalog no.",
  country: "Country",
  barcode: "Barcode",
  cover: "Cover",
  performers: "Performers",
  credits: "Other credits",
};

/** Fields a person can type a value for (and so freeze). */
export function isTextField(field: AlbumField): boolean {
  return field !== "cover" && field !== "performers" && field !== "credits";
}

export const FACT_SOURCE_LABELS: Record<FactSource, string> = {
  tags: "File tags",
  online: "MusicBrainz",
};

/** One fact of an album, when the server reports sources. */
export function resolvedFact(
  about: AlbumAbout | undefined | null,
  field: AlbumField,
): ResolvedFact | null {
  return about?.sources?.facts.find((fact) => fact.field === field) ?? null;
}

/** "Edited", "File tags" or "MusicBrainz", noting a fallback. */
export function factSourceLabel(fact: ResolvedFact): string | null {
  if (!fact.source) return null;
  if (fact.source === "edited") return "Edited";
  const label = FACT_SOURCE_LABELS[fact.source];
  return fact.fallback
    ? `${label} (no ${FACT_SOURCE_LABELS[fact.preferred].toLowerCase()} value)`
    : label;
}

/** The year the album hero shows: the original release, not the edition. */
export function albumHeroYear(
  about: AlbumAbout | undefined | null,
  tagDate: string | null | undefined,
): string | null {
  const year = about?.sources?.hero_year;
  if (year) return year;
  const fallback = tagDate?.slice(0, 4) ?? "";
  return /^\d{4}$/.test(fallback) ? fallback : null;
}

/** The value a field choice sends: the album's own source or the default. */
export type FieldChoice = FactSource | "default";

export function fieldChoice(fact: ResolvedFact): FieldChoice {
  return fact.choice ?? "default";
}

/** Whether any field of the album is frozen or has its own source. */
export function hasSourceOverrides(about: AlbumAbout | undefined | null) {
  const sources = about?.sources;
  if (!sources) return false;
  return (
    sources.credits_edited ||
    sources.facts.some((fact) => fact.choice !== null || fact.edited !== null)
  );
}

/**
 * The album's credits as an editable list: one entry per person, role and
 * instrument, keeping the tracks each applies to and the linked person.
 */
export function editableCredits(credits: AboutCredit[]): EditedCredit[] {
  return credits.map((credit) => ({
    name: credit.name,
    role: credit.role,
    instrument: credit.instrument,
    track_ids: [...credit.track_ids],
    artist_id: credit.artist_id,
  }));
}

/** Drops empty rows and trims names before saving an edited list. */
export function cleanEditedCredits(credits: EditedCredit[]): EditedCredit[] {
  return credits
    .map((credit) => ({
      ...credit,
      name: credit.name.trim(),
      instrument:
        credit.role === "performer" && credit.instrument?.trim()
          ? credit.instrument.trim().toLowerCase()
          : null,
    }))
    .filter((credit) => credit.name.length > 0);
}

/** Identify results narrowed to releases crediting the tagged people. */
export function filterIdentifyReleases(
  releases: ReleaseMatch[],
  only: { composer?: boolean; conductor?: boolean },
): ReleaseMatch[] {
  return releases.filter(
    (release) =>
      (!only.composer || release.credits_tagged_composer === true) &&
      (!only.conductor || release.credits_tagged_conductor === true),
  );
}

/** Unsaved changes in the "Metadata preference" editor. */
export interface SourcesDraft {
  sources: Partial<Record<AlbumField, FieldChoice>>;
  /** Typed values; an empty string unfreezes the field. */
  edits: Partial<Record<AlbumField, string>>;
}

/**
 * The changes a draft makes to an album's sources, or null when it makes
 * none: a choice equal to the current one and an unchanged value are left
 * out.
 */
export function sourcesPatch(
  facts: ResolvedFact[],
  draft: SourcesDraft,
): AlbumSourcesPatch | null {
  const patch: AlbumSourcesPatch = { sources: {}, edits: {} };
  let changed = false;
  for (const fact of facts) {
    const choice = draft.sources[fact.field];
    if (choice !== undefined && choice !== fieldChoice(fact)) {
      patch.sources![fact.field] = choice === "default" ? null : choice;
      changed = true;
    }
    const typed = draft.edits[fact.field];
    if (typed === undefined || !isTextField(fact.field)) continue;
    const value = typed.trim();
    if (value === (fact.edited ?? "")) continue;
    patch.edits![fact.field] = value || null;
    changed = true;
  }
  return changed ? patch : null;
}
