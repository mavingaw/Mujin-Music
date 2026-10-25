import { expect, test } from "bun:test";
import type {
  ComposerWork,
  EditorialText,
  PersonRef,
  TrackWork,
  WorkRecording,
  WorkSummary,
} from "../domain/types";
import {
  albumTextPanels,
  albumWorkSections,
  composerWorkFacets,
  filterComposerWorks,
  groupRecordings,
  groupWorksByForm,
  pluralForm,
  recordingPerformers,
  trackWorkTarget,
  workFacts,
} from "./works";

const work = (overrides: Partial<WorkSummary>): WorkSummary => ({
  mbid: "w",
  title: "Work",
  form: null,
  key: null,
  catalogue: null,
  composed: null,
  period: null,
  composer: null,
  language: null,
  parent_mbid: null,
  part_position: null,
  ...overrides,
});

const composerWork = (overrides: Partial<WorkSummary>): ComposerWork => ({
  work: work(overrides),
  track_count: 1,
  recording_count: 1,
});

const ninth = work({
  mbid: "ninth",
  title: "Symphony No. 9 in D minor, Op. 125",
  form: "Symphony",
  key: "D minor",
  catalogue: "Op. 125",
  composed: "1822–1824",
  period: "Classical",
});

test("forms are pluralised for composer page headings", () => {
  expect(pluralForm("Symphony")).toBe("Symphonies");
  expect(pluralForm("Sonata")).toBe("Sonatas");
  expect(pluralForm("Mass")).toBe("Masses");
  expect(pluralForm("Variations")).toBe("Variations");
  expect(pluralForm("String quartet")).toBe("String quartets");
});

test("work facts skip what the title already says", () => {
  expect(workFacts(ninth)).toEqual([
    "Symphony",
    "composed 1822–1824",
    "Classical",
  ]);
  expect(
    workFacts(
      work({ title: "Für Elise", key: "A minor", catalogue: "WoO 59" }),
    ),
  ).toEqual(["A minor", "WoO 59"]);
});

test("composer works filter by form, period and key and group by form", () => {
  const works = [
    composerWork({ ...ninth }),
    composerWork({
      mbid: "fifth",
      title: "Symphony No. 5",
      form: "Symphony",
      key: "C minor",
      catalogue: "Op. 67",
      period: "Classical",
    }),
    composerWork({
      mbid: "sonata",
      title: "Piano Sonata No. 32",
      form: "Sonata",
      key: "C minor",
      catalogue: "Op. 111",
      period: "Classical",
    }),
    composerWork({ mbid: "song", title: "Untitled", period: "Romantic" }),
  ];
  const facets = composerWorkFacets(works);
  expect(facets.forms).toEqual([
    { value: "Symphony", count: 2 },
    { value: "Sonata", count: 1 },
  ]);
  expect(facets.periods.map((facet) => facet.value)).toEqual([
    "Classical",
    "Romantic",
  ]);
  expect(facets.keys).toEqual([
    { value: "C minor", count: 2 },
    { value: "D minor", count: 1 },
  ]);
  expect(
    filterComposerWorks(works, { key: "C minor" }).map(
      (entry) => entry.work.mbid,
    ),
  ).toEqual(["fifth", "sonata"]);
  expect(
    filterComposerWorks(works, { form: "Symphony", key: "C minor" }).map(
      (entry) => entry.work.mbid,
    ),
  ).toEqual(["fifth"]);
  const groups = groupWorksByForm(works);
  expect(groups.map((group) => group.label)).toEqual([
    "Symphonies",
    "Sonatas",
    "Other works",
  ]);
  // Catalogue order inside a form: Op. 67 before Op. 125.
  expect(groups[0].works.map((entry) => entry.work.mbid)).toEqual([
    "fifth",
    "ninth",
  ]);
});

const person = (name: string, id: string | null = name): PersonRef => ({
  artist_id: id,
  name,
});

const recording = (
  album: string,
  people: Partial<WorkRecording>,
): WorkRecording =>
  ({
    album: { id: album, name: album },
    artist: { id: "a", name: "a" },
    year: null,
    conductors: [],
    ensembles: [],
    soloists: [],
    artists: [],
    tracks: [],
    complete: true,
    duration: 0,
    ...people,
  }) as unknown as WorkRecording;

test("recordings group by conductor and by performer", () => {
  const karajan63 = recording("1963", {
    conductors: [person("Herbert von Karajan")],
    ensembles: [person("Berliner Philharmoniker")],
  });
  const karajan77 = recording("1977", {
    conductors: [person("Herbert von Karajan")],
    ensembles: [person("Berliner Philharmoniker")],
    soloists: [{ ...person("Anna Tomowa-Sintow"), instrument: "soprano" }],
  });
  const piano = recording("Liszt transcription", {
    artists: [person("Cyprien Katsaris")],
  });
  const recordings = [karajan63, karajan77, piano];
  const byConductor = groupRecordings(recordings, "conductor");
  expect(
    byConductor.map((group) => [group.label, group.recordings.length]),
  ).toEqual([
    ["Herbert von Karajan", 2],
    ["No conductor credited", 1],
  ]);
  const byPerformer = groupRecordings(recordings, "performer");
  expect(
    byPerformer.map((group) => [group.label, group.recordings.length]),
  ).toEqual([
    ["Berliner Philharmoniker", 2],
    ["Anna Tomowa-Sintow", 1],
    ["Cyprien Katsaris", 1],
  ]);
  expect(groupRecordings(recordings, "album")).toHaveLength(3);
  expect(recordingPerformers(karajan77)).toBe(
    "Berliner Philharmoniker · Anna Tomowa-Sintow (soprano) · Herbert von Karajan, conductor",
  );
});

test("consecutive movements of one piece share an album section", () => {
  const movement = (track: string, parent: WorkSummary | null): TrackWork => ({
    track_id: track,
    work: work({ mbid: `${track}-work` }),
    parent,
    part_title: track,
  });
  const works = [
    movement("t1", ninth),
    movement("t2", ninth),
    movement("t3", null),
    movement("t4", ninth),
  ];
  const sections = albumWorkSections(["t1", "t2", "t3", "t4", "t5"], works);
  expect(
    sections.map((section) => [section.work?.mbid ?? null, section.trackIds]),
  ).toEqual([
    ["ninth", ["t1", "t2"]],
    [null, ["t3"]],
    ["ninth", ["t4"]],
    [null, ["t5"]],
  ]);
  expect(trackWorkTarget(works)?.mbid).toBe("ninth");
  expect(trackWorkTarget([movement("t3", null)])?.mbid).toBe("t3-work");
  expect(trackWorkTarget([])).toBeNull();
});

const wikiText = (
  title: string,
  kind: EditorialText["kind"] = "background",
): EditorialText => ({
  kind,
  source: "wikipedia",
  language: "en",
  title,
  body: `${title} text`,
  url: `https://en.wikipedia.org/wiki/${title.replace(/ /g, "_")}`,
  license: "CC BY-SA 4.0",
  license_url: "https://creativecommons.org/licenses/by-sa/4.0/",
});

test("album text panels put the review first and add the pieces' articles", () => {
  expect(albumTextPanels(null)).toEqual([]);
  const jazz = albumTextPanels({
    review: wikiText("Kind of Blue", "reception"),
    background: wikiText("Kind of Blue"),
  });
  expect(jazz.map((panel) => panel.label)).toEqual(["Review", "About"]);

  // A classical album without an article of its own shows its piece.
  const classical = albumTextPanels({
    review: null,
    background: null,
    work_texts: [
      {
        work_mbid: "ninth",
        title: "Symphony No. 9 in D minor, Op. 125",
        text: wikiText("Symphony No. 9 (Beethoven)"),
      },
    ],
  });
  expect(classical).toHaveLength(1);
  expect(classical[0]).toMatchObject({
    key: "work:ninth",
    label: "About the work",
    workMbid: "ninth",
  });

  // Several pieces are named; one that is the album's own article is not repeated.
  const coupling = albumTextPanels({
    review: null,
    background: wikiText("Symphony No. 9 (Beethoven)"),
    work_texts: [
      {
        work_mbid: "ninth",
        title: "Symphony No. 9",
        text: wikiText("Symphony No. 9 (Beethoven)"),
      },
      {
        work_mbid: "eighth",
        title: "Symphony No. 8",
        text: wikiText("Symphony No. 8 (Beethoven)"),
      },
    ],
  });
  expect(coupling.map((panel) => panel.label)).toEqual([
    "About",
    "Symphony No. 8",
  ]);
});
