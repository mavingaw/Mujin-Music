import { describe, expect, test } from "bun:test";
import {
  enrichmentNoMatch,
  enrichmentStageLine,
  enrichmentWaitingLabel,
  personHue,
  personTone,
  PERSON_TONES,
  personInitials,
} from "./people";

describe("personInitials", () => {
  test("uses the first and last meaningful words", () => {
    expect(personInitials("Ludwig van Beethoven")).toBe("LB");
    expect(personInitials("Herbert von Karajan")).toBe("HK");
    expect(personInitials("Carlos Kleiber")).toBe("CK");
    expect(personInitials("Wiener Philharmoniker")).toBe("WP");
    expect(personInitials("Johann Sebastian Bach")).toBe("JB");
  });

  test("handles single names, accents, brackets and empty names", () => {
    expect(personInitials("björk")).toBe("B");
    expect(personInitials("Édith Piaf")).toBe("ÉP");
    expect(personInitials("The Beatles")).toBe("TB");
    expect(personInitials("Karajan (conductor)")).toBe("K");
    expect(personInitials("  ")).toBe("?");
    expect(personInitials("!!!")).toBe("?");
    expect(personInitials("Anne (née Smith) Mutter (violin)")).toBe("AM");
    // An unclosed bracket is kept as written.
    expect(personInitials("Anne (Sophie Mutter")).toBe("AM");
  });

  test("long adversarial names finish in linear time", () => {
    const started = performance.now();
    expect(personInitials(`${"(".repeat(100_000)}x`)).toBe("X");
    expect(personInitials(`a${" (b".repeat(50_000)}`)).toBe("AB");
    expect(performance.now() - started).toBeLessThan(1_000);
  });

  test("gives each person a stable hue", () => {
    expect(personHue("Carlos Kleiber")).toBe(personHue("carlos kleiber"));
    expect(personHue("Carlos Kleiber")).toBeGreaterThanOrEqual(0);
    expect(personHue("Carlos Kleiber")).toBeLessThan(360);
    expect(PERSON_TONES).toContainEqual(personTone("Carlos Kleiber"));
    expect(personTone("Carlos Kleiber")).toEqual(personTone("carlos kleiber"));
  });
});

describe("enrichmentWaitingLabel", () => {
  const task = (stage?: string, status = "pending") => ({
    status,
    last_error: null,
    updated_at: "2026-10-25 06:00:00",
    stage,
  });

  test("names the stage the lookup waits for", () => {
    expect(enrichmentWaitingLabel(task("wikipedia"), "artist")).toBe(
      "Fetching the biography from Wikipedia.",
    );
    expect(enrichmentWaitingLabel(task(), "album")).toBe(
      "Looking this album up on MusicBrainz.",
    );
    expect(enrichmentWaitingLabel(task("commons", "done"), "artist")).toBe(
      null,
    );
    expect(enrichmentWaitingLabel(null, "artist")).toBe(null);
  });

  test("says when a busy host will be asked again", () => {
    expect(
      enrichmentWaitingLabel(task("commons"), "artist", [
        {
          stage: "commons",
          host: "commons.wikimedia.org",
          counts: { pending: 3, done: 0, skipped: 0, failed: 0 },
          paused_seconds: 125,
        },
      ]),
    ).toBe(
      "Fetching the photo from Wikimedia Commons; the service is busy, retrying in 2 min.",
    );
  });
});

describe("enrichmentStageLine", () => {
  test("summarises a queue and whether its host is busy", () => {
    expect(
      enrichmentStageLine({
        stage: "commons",
        host: "commons.wikimedia.org",
        counts: { pending: 3, done: 12, skipped: 1, failed: 0 },
        paused_seconds: 60,
      }),
    ).toEqual({
      label: "Wikimedia Commons (artist photos)",
      detail: "3 queued · 12 done · busy, retrying in 1 min",
      busy: true,
    });
    expect(
      enrichmentStageLine({
        stage: "wikipedia",
        host: "wikipedia.org",
        counts: { pending: 0, done: 4, skipped: 0, failed: 2 },
        paused_seconds: 0,
      }).detail,
    ).toBe("4 done · 2 failed");
    expect(
      enrichmentStageLine({
        stage: "coverart",
        host: "coverartarchive.org",
        counts: { pending: 0, done: 5, skipped: 1, failed: 0 },
        paused_seconds: 0,
      }).label,
    ).toBe("Cover Art Archive (album covers)");
    expect(
      enrichmentStageLine({
        stage: "relations",
        host: "query.wikidata.org",
        counts: { pending: 1, done: 0, skipped: 0, failed: 0 },
        paused_seconds: 0,
      }).label,
    ).toBe("Wikidata Query Service (influences and followers)");
  });
});

describe("enrichmentNoMatch", () => {
  const skipped = (last_error: string | null) => ({
    status: "skipped",
    last_error,
    updated_at: "2026-10-25 06:00:00",
  });

  test("reports a real no-match with its reason", () => {
    expect(enrichmentNoMatch(skipped("no release scored above 0.8"))).toEqual({
      reason: "no release scored above 0.8",
    });
    expect(enrichmentNoMatch(skipped(null))).toEqual({ reason: null });
  });

  test("never claims a present album left the library", () => {
    expect(
      enrichmentNoMatch(skipped("the album is no longer in the library")),
    ).toBe(null);
    expect(
      enrichmentNoMatch(skipped("the library is still loading this album")),
    ).toBe(null);
    expect(enrichmentNoMatch({ ...skipped(null), status: "pending" })).toBe(
      null,
    );
    expect(enrichmentNoMatch(undefined)).toBe(null);
  });
});
