import { expect, test } from "bun:test";
import {
  composerInitials,
  composerSummary,
  personRoles,
} from "./composer-summary";

test("composer rows summarise compositions, albums and extra tracks", () => {
  expect(composerSummary({ workCount: 1, albumCount: 1, trackCount: 1 })).toBe(
    "1 composition · 1 album",
  );
  expect(
    composerSummary({ workCount: 12, albumCount: 3, trackCount: 40 }),
  ).toBe("12 compositions · 3 albums · 40 tracks");
  expect(composerSummary({ workCount: 2, albumCount: 0, trackCount: 2 })).toBe(
    "2 compositions",
  );
});

test("initials use the first and last name", () => {
  expect(composerInitials("Johann Sebastian Bach")).toBe("JB");
  expect(composerInitials("Björk")).toBe("B");
  expect(composerInitials("  ")).toBe("?");
  expect(composerInitials("Rimsky-Korsakov")).toBe("RK");
});

test("person roles read in credit column order", () => {
  expect(
    personRoles({
      roles: [
        { role: "primary", track_count: 2 },
        { role: "conductor_ensemble", track_count: 1 },
        { role: "composer", track_count: 4 },
      ],
    }),
  ).toBe("Artist · Conductor / Ensemble · Composer");
});
