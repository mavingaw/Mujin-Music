import type {
  AlbumAbout,
  ComposerWork,
  ComposerWorks,
  EditorialText,
  PersonRef,
  TrackWork,
  WorkRecording,
  WorkSummary,
} from "../domain/types";

/** Style periods in time order. */
export const PERIOD_ORDER = [
  "Medieval",
  "Renaissance",
  "Baroque",
  "Classical",
  "Romantic",
  "Modern",
  "Contemporary",
] as const;

const PLURAL_FORMS: Record<string, string> = {
  Mass: "Masses",
  Variations: "Variations",
  "Incidental music": "Incidental music",
  Soundtrack: "Soundtracks",
};

/** "Symphony" -> "Symphonies", "Sonata" -> "Sonatas", "Mass" -> "Masses". */
export function pluralForm(form: string) {
  if (PLURAL_FORMS[form]) return PLURAL_FORMS[form];
  if (/[^aeiou]y$/i.test(form)) return `${form.slice(0, -1)}ies`;
  if (/(s|x|ch|sh)$/i.test(form)) return `${form}es`;
  return `${form}s`;
}

/** Short facts under a work title: form, key, catalogue, year and period.
 * A catalogue number or key already in the title is not repeated. */
export function workFacts(work: WorkSummary) {
  const title = work.title.toLowerCase();
  const facts: string[] = [];
  if (work.form) facts.push(work.form);
  if (work.key && !title.includes(work.key.toLowerCase())) facts.push(work.key);
  if (work.catalogue && !title.includes(work.catalogue.toLowerCase())) {
    facts.push(work.catalogue);
  }
  if (work.composed) facts.push(`composed ${work.composed}`);
  if (work.period) facts.push(work.period);
  return facts;
}

/** The URL path of a work page. */
export function workHref(mbid: string) {
  return `/work?id=${encodeURIComponent(mbid)}`;
}

export interface WorkFacetValue {
  value: string;
  count: number;
}

export interface WorkFacets {
  forms: WorkFacetValue[];
  periods: WorkFacetValue[];
  keys: WorkFacetValue[];
}

function countValues(values: (string | null | undefined)[]) {
  const counts = new Map<string, number>();
  for (const value of values) {
    if (value) counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  return [...counts].map(([value, count]) => ({ value, count }));
}

/** Filter chip values for a composer's works. */
export function composerWorkFacets(works: readonly ComposerWork[]): WorkFacets {
  const periodRank = (value: string) => {
    const index = PERIOD_ORDER.indexOf(value as (typeof PERIOD_ORDER)[number]);
    return index === -1 ? PERIOD_ORDER.length : index;
  };
  return {
    forms: countValues(works.map((entry) => entry.work.form)).sort(
      (left, right) =>
        right.count - left.count || left.value.localeCompare(right.value),
    ),
    periods: countValues(works.map((entry) => entry.work.period)).sort(
      (left, right) => periodRank(left.value) - periodRank(right.value),
    ),
    keys: countValues(works.map((entry) => entry.work.key)).sort(
      (left, right) => left.value.localeCompare(right.value),
    ),
  };
}

/** Which part of a composer's catalogue a list shows. */
export type CatalogueScope = "all" | "library" | "notable";

export interface WorkFilter {
  form?: string | null;
  period?: string | null;
  key?: string | null;
  /** All works (default), those in the library, or the notable ones. */
  scope?: CatalogueScope | null;
}

export function filterComposerWorks(
  works: readonly ComposerWork[],
  filter: WorkFilter,
) {
  return works.filter(
    (entry) =>
      (!filter.form || entry.work.form === filter.form) &&
      (!filter.period || entry.work.period === filter.period) &&
      (!filter.key || entry.work.key === filter.key) &&
      (filter.scope !== "library" || entry.track_count > 0) &&
      (filter.scope !== "notable" || isNotable(entry)),
  );
}

/** Whether a work is one of the composer's notable works: Wikidata names
 * it, or (before the catalogue is read) it is in the library. */
export function isNotable(entry: ComposerWork) {
  return entry.notable_rank != null;
}

/**
 * The composer's notable works, Roon's "Notable compositions": the works
 * Wikidata lists as notable, in its order, then up to `limit` in all with
 * the works most recorded in the library. A composer whose catalogue was
 * not read yet shows their most recorded works.
 */
export function notableWorks(works: readonly ComposerWork[], limit = 12) {
  const ranked = works
    .filter(isNotable)
    .sort(
      (left, right) => (left.notable_rank ?? 0) - (right.notable_rank ?? 0),
    );
  const recorded = works
    .filter((entry) => !isNotable(entry) && entry.track_count > 0)
    .sort(
      (left, right) =>
        right.recording_count - left.recording_count ||
        right.track_count - left.track_count ||
        left.work.title.localeCompare(right.work.title),
    );
  return [...ranked, ...recorded].slice(0, limit);
}

/** "Composed 1804–1808" under a work, when the year is known. */
export function composedLine(work: Pick<WorkSummary, "composed">) {
  return work.composed ? `Composed ${work.composed}` : null;
}

/** "In your library · 2 recordings" or "Not in your library". */
export function libraryLine(entry: ComposerWork) {
  if (entry.track_count <= 0) return "Not in your library";
  const recordings = entry.recording_count || 1;
  return `In your library · ${recordings} ${recordings === 1 ? "recording" : "recordings"}`;
}

/** The headline over a composer's catalogue. */
export function catalogueHeadline(
  works: readonly ComposerWork[],
  catalogue: ComposerWorks["catalogue"],
) {
  const inLibrary = works.filter((entry) => entry.track_count > 0).length;
  const pieces = works.length;
  const parts = [
    `${pieces.toLocaleString("en-US")} ${pieces === 1 ? "work" : "works"}`,
    `${inLibrary.toLocaleString("en-US")} in your library`,
  ];
  if (catalogue?.notable) parts.push(`${catalogue.notable} notable`);
  return parts.join(" · ");
}

/** What the catalogue note says while it is not read (or not complete). */
export function catalogueStatusText(catalogue: ComposerWorks["catalogue"]) {
  if (!catalogue) return null;
  switch (catalogue.state) {
    case "pending":
      return "Reading the full catalogue from MusicBrainz…";
    case "failed":
      return "MusicBrainz did not answer; the full catalogue will be read later.";
    case "off":
      return "Turn on online metadata to see every work this composer wrote, not only those in your library.";
    case "none":
      return null;
    default:
      return catalogue.complete
        ? null
        : `MusicBrainz lists ${catalogue.musicbrainz_total.toLocaleString("en-US")} works (movements included); Mujin Music shows the first 4,000.`;
  }
}

export interface WorkFormGroup {
  form: string | null;
  label: string;
  works: ComposerWork[];
}

/** Notable compositions grouped by form, largest group first, the way a
 * composer page lists Symphonies, Concertos, Sonatas, ... Within a group
 * works follow catalogue order when they have one. */
export function groupWorksByForm(
  works: readonly ComposerWork[],
): WorkFormGroup[] {
  const groups = new Map<string, ComposerWork[]>();
  for (const entry of works) {
    const key = entry.work.form ?? "";
    groups.set(key, [...(groups.get(key) ?? []), entry]);
  }
  const catalogueNumber = (work: WorkSummary) => {
    const match = work.catalogue?.match(/\d+/);
    return match ? Number(match[0]) : Number.POSITIVE_INFINITY;
  };
  return [...groups]
    .map(([form, entries]) => ({
      form: form || null,
      label: form ? pluralForm(form) : "Other works",
      works: [...entries].sort(
        (left, right) =>
          catalogueNumber(left.work) - catalogueNumber(right.work) ||
          left.work.title.localeCompare(right.work.title, undefined, {
            numeric: true,
          }),
      ),
    }))
    .sort(
      (left, right) =>
        Number(left.form === null) - Number(right.form === null) ||
        right.works.length - left.works.length ||
        left.label.localeCompare(right.label),
    );
}

export type RecordingGrouping = "album" | "conductor" | "performer";

export interface RecordingGroup {
  key: string;
  label: string;
  person: PersonRef | null;
  recordings: WorkRecording[];
}

function personKey(person: PersonRef) {
  return person.artist_id ?? `name:${person.name.toLowerCase()}`;
}

/** Recordings of a work grouped by album, by conductor or by performer
 * (orchestra, ensemble, soloists and track artists). A recording with two
 * performers appears under both. */
export function groupRecordings(
  recordings: readonly WorkRecording[],
  by: RecordingGrouping,
): RecordingGroup[] {
  if (by === "album") {
    return recordings.map((recording) => ({
      key: recording.album.id,
      label: recording.album.name,
      person: null,
      recordings: [recording],
    }));
  }
  const groups = new Map<string, RecordingGroup>();
  const unknown: WorkRecording[] = [];
  for (const recording of recordings) {
    const people =
      by === "conductor"
        ? recording.conductors
        : [...recording.ensembles, ...recording.soloists, ...recording.artists];
    if (people.length === 0) {
      unknown.push(recording);
      continue;
    }
    for (const person of people) {
      const key = personKey(person);
      const group = groups.get(key) ?? {
        key,
        label: person.name,
        person,
        recordings: [],
      };
      if (!group.recordings.includes(recording)) {
        group.recordings.push(recording);
      }
      groups.set(key, group);
    }
  }
  const sorted = [...groups.values()].sort(
    (left, right) =>
      right.recordings.length - left.recordings.length ||
      left.label.localeCompare(right.label),
  );
  if (unknown.length > 0) {
    sorted.push({
      key: "unknown",
      label:
        by === "conductor" ? "No conductor credited" : "No performer credited",
      person: null,
      recordings: unknown,
    });
  }
  return sorted;
}

/** The people line for a recording: "Berliner Philharmoniker · Herbert von
 * Karajan, conductor". */
export function recordingPerformers(recording: WorkRecording) {
  const names = [
    ...recording.ensembles.map((person) => person.name),
    ...recording.soloists.map((person) =>
      person.instrument ? `${person.name} (${person.instrument})` : person.name,
    ),
    ...recording.artists.map((person) => person.name),
  ];
  const conductors = recording.conductors.map(
    (person) => `${person.name}, conductor`,
  );
  return [...names, ...conductors].join(" · ");
}

export interface AlbumWorkSection {
  /** The piece the tracks belong to, or null for tracks without one. */
  work: WorkSummary | null;
  trackIds: string[];
}

/** Splits an album's track order into sections: consecutive movements of
 * one piece share a heading; everything else stands alone. */
export function albumWorkSections(
  trackIds: readonly string[],
  works: readonly TrackWork[] | undefined,
): AlbumWorkSection[] {
  const byTrack = new Map(
    (works ?? []).map((entry) => [entry.track_id, entry]),
  );
  const sections: AlbumWorkSection[] = [];
  for (const id of trackIds) {
    const piece = byTrack.get(id)?.parent ?? null;
    const last = sections.at(-1);
    if (piece && last?.work?.mbid === piece.mbid) {
      last.trackIds.push(id);
    } else {
      sections.push({ work: piece, trackIds: [id] });
    }
  }
  return sections;
}

/** The work a track leads to with "Go to work": the piece for a movement,
 * otherwise the work itself. */
export function trackWorkTarget(works: readonly TrackWork[] | undefined) {
  const first = works?.[0];
  if (!first) return null;
  return first.parent ?? first.work;
}

/** One tab of an album's text switcher. */
export interface AlbumTextPanel {
  key: string;
  label: string;
  text: EditorialText;
  /** The piece the text is about, for a link to its page. */
  workMbid?: string;
  workTitle?: string;
}

/**
 * The album's text tabs, Roon's Review / About switcher: the Wikipedia
 * reception section, the album's own article and then the articles of the
 * pieces on it (what a classical album has when the release has no
 * article). A piece's article that is also the album's is not repeated.
 */
export function albumTextPanels(
  about:
    | Pick<AlbumAbout, "review" | "background" | "work_texts" | "text_sources">
    | null
    | undefined,
  sourceId?: string | null,
): AlbumTextPanel[] {
  if (!about) return [];
  const panels: AlbumTextPanel[] = [];
  // The chosen editorial source (another Wikipedia) when there is one.
  const source = sourceId
    ? about.text_sources?.find((candidate) => candidate.id === sourceId)
    : undefined;
  const review = source ? source.review : about.review;
  const background = source ? source.background : about.background;
  if (review) panels.push({ key: "review", label: "Review", text: review });
  if (background)
    panels.push({ key: "background", label: "About", text: background });
  const works = (about.work_texts ?? []).filter(
    (work) =>
      work.text.url !== background?.url &&
      work.text.url !== about.background?.url,
  );
  for (const work of works) {
    panels.push({
      key: `work:${work.work_mbid}`,
      label: works.length === 1 && !background ? "About the work" : work.title,
      text: work.text,
      workMbid: work.work_mbid,
      workTitle: work.title,
    });
  }
  return panels;
}
