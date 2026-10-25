import type {
  ComposerWork,
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

export interface FacetValue {
  value: string;
  count: number;
}

export interface WorkFacets {
  forms: FacetValue[];
  periods: FacetValue[];
  keys: FacetValue[];
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

export interface WorkFilter {
  form?: string | null;
  period?: string | null;
  key?: string | null;
}

export function filterComposerWorks(
  works: readonly ComposerWork[],
  filter: WorkFilter,
) {
  return works.filter(
    ({ work }) =>
      (!filter.form || work.form === filter.form) &&
      (!filter.period || work.period === filter.period) &&
      (!filter.key || work.key === filter.key),
  );
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
