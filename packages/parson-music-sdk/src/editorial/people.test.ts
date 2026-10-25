import { describe, expect, test } from "bun:test";
import {
  enrichmentStageLine,
  enrichmentWaitingLabel,
  personHue,
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
  });

  test("gives each person a stable hue", () => {
    expect(personHue("Carlos Kleiber")).toBe(personHue("carlos kleiber"));
    expect(personHue("Carlos Kleiber")).toBeGreaterThanOrEqual(0);
    expect(personHue("Carlos Kleiber")).toBeLessThan(360);
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
  });
});
