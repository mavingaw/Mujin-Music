import { describe, expect, test } from "bun:test";
import type { AboutCredit } from "../domain/types";
import {
  albumFormat,
  creditRoleLabel,
  creditRolesLabel,
  creditTracksLabel,
  creditsForTrack,
  formatAudio,
  formatPartialDate,
  groupCredits,
  lifeSpanFacts,
  paragraphs,
  trackCreditLine,
} from "./credits";

const credit = (
  name: string,
  role: string,
  instrument: string | null = null,
  track_ids: string[] = [],
  artist_id: string | null = name.toLowerCase(),
): AboutCredit => ({
  artist_id,
  name,
  role,
  instrument,
  track_ids,
  sources: ["tags"],
});

describe("credit labels", () => {
  test("roles and instruments read like liner notes", () => {
    expect(creditRoleLabel("producer")).toBe("Producer");
    expect(creditRoleLabel("producer", null, true)).toBe("Producers");
    expect(creditRoleLabel("performer", "piano")).toBe("Piano");
    expect(creditRoleLabel("chorus_master")).toBe("Chorus master");
    expect(
      creditRolesLabel([
        { role: "producer", instrument: null },
        { role: "composer", instrument: null },
      ]),
    ).toBe("Composer & Producer");
  });

  test("credits group by role, then instrument, without duplicates", () => {
    const groups = groupCredits([
      credit("Teo Macero", "producer"),
      credit("Bill Evans", "performer", "piano"),
      credit("Miles Davis", "composer"),
      credit("Bill Evans", "composer"),
      credit("Jimmy Cobb", "performer", "drums"),
      credit("Bill Evans", "performer", "piano", ["2"]),
    ]);
    expect(groups.map((group) => [group.label, group.credits.length])).toEqual([
      ["Composers", 2],
      ["Drums", 1],
      ["Piano", 1],
      ["Producer", 1],
    ]);
  });

  test("tracks show a composed-by line and per-track credits", () => {
    const credits = [
      credit("Miles Davis", "composer", null, ["a", "b"]),
      credit("Bill Evans", "composer", null, ["b"]),
      credit("Teo Macero", "producer", null, []),
    ];
    expect(trackCreditLine(creditsForTrack(credits, "b"))).toEqual({
      label: "Composed by",
      people: [
        { id: "miles davis", name: "Miles Davis" },
        { id: "bill evans", name: "Bill Evans" },
      ],
    });
    expect(trackCreditLine(creditsForTrack(credits, "c"))).toBeNull();
    expect(trackCreditLine([credit("Hal David", "lyricist")])?.label).toBe(
      "Lyrics by",
    );
  });

  test("track ranges are compact", () => {
    const tracks = [1, 2, 3, 4, 5].map((number) => ({
      id: `t${number}`,
      track_number: number,
    }));
    expect(creditTracksLabel([], tracks)).toBe("All tracks");
    expect(creditTracksLabel(["t1", "t2", "t3", "t4", "t5"], tracks)).toBe(
      "All tracks",
    );
    expect(creditTracksLabel(["t3"], tracks)).toBe("Track 3");
    expect(creditTracksLabel(["t1", "t2", "t3", "t5"], tracks)).toBe(
      "Tracks 1–3, 5",
    );
    expect(creditTracksLabel(["t1", "t2", "t5"], tracks)).toBe(
      "Tracks 1, 2, 5",
    );
  });
});

describe("facts and formats", () => {
  test("partial dates and life spans", () => {
    expect(formatPartialDate("1959-08-17")).toBe("17 August 1959");
    expect(formatPartialDate("1959-08")).toBe("August 1959");
    expect(formatPartialDate("1959")).toBe("1959");
    expect(formatPartialDate("")).toBeNull();
    expect(
      lifeSpanFacts({
        kind: "Person",
        begin: "1926-05-26",
        end: "1991-09-28",
        begin_area: "Alton",
        area: "United States",
      }),
    ).toEqual([
      { label: "Born", value: "26 May 1926 in Alton" },
      { label: "Died", value: "28 September 1991" },
      { label: "From", value: "United States" },
    ]);
    expect(lifeSpanFacts({ kind: "Group", begin: "1960" })).toEqual([
      { label: "Formed", value: "1960" },
    ]);
  });

  test("audio formats summarise lossless and lossy files", () => {
    const flac = {
      codec: "flac",
      container: "flac",
      sample_rate: 96000,
      bit_depth: 24,
      channels: 2,
      bitrate_kbps: 2800,
      lossless: true,
    };
    const mp3 = {
      ...flac,
      codec: "mp3",
      sample_rate: 44100,
      bit_depth: null,
      bitrate_kbps: 320,
      lossless: false,
    };
    expect(formatAudio(flac)).toBe("FLAC 96 kHz 24-bit");
    expect(
      formatAudio({
        ...flac,
        codec: "pcm",
        sample_rate: 22_050,
        bit_depth: 16,
      }),
    ).toBe("PCM 22.05 kHz 16-bit");
    expect(formatAudio({ ...flac, sample_rate: 44100 })).toBe(
      "FLAC 44.1 kHz 24-bit",
    );
    expect(formatAudio(mp3)).toBe("MP3 320 kbps");
    expect(
      formatAudio({ ...flac, sample_rate: 44100, enhanced_by: "Apollo" }),
    ).toBe("AI-enhanced (Apollo), FLAC 44.1 kHz");
    expect(
      albumFormat([{ audio: flac }, { audio: flac }, { audio: mp3 }]),
    ).toBe("FLAC 96 kHz 24-bit");
    expect(albumFormat([{ audio: flac }, { audio: mp3 }])).toBe(
      "Mixed formats",
    );
    expect(albumFormat([{ audio: null }])).toBeNull();
    // Variable-bitrate Opus files are one format, not "Mixed formats".
    const opus = (bitrate_kbps: number) => ({
      audio: { ...mp3, codec: "opus", bitrate_kbps },
    });
    expect(albumFormat([opus(128), opus(131), opus(119), opus(140)])).toBe(
      "Opus ~130 kbps",
    );
  });

  test("editorial text splits into paragraphs", () => {
    expect(paragraphs("One.\n\nTwo.\n\n\n  \n\nThree.")).toEqual([
      "One.",
      "Two.",
      "Three.",
    ]);
  });
});
