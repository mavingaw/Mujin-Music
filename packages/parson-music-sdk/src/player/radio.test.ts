import { expect, test } from "bun:test";
import {
  radioComingUpIn,
  radioQueueHistory,
  radioSectionStart,
  radioSeedKey,
  radioSkipReasonEvent,
  radioSkipReasons,
  radioStationLabel,
  shouldContinueWithRadio,
  upcomingRadioPicks,
} from "./radio";

test("the Radio section is the trailing run of picks after the current item", () => {
  const origins = ["manual", "manual", "generated", "generated"] as const;
  expect(radioSectionStart(origins, 0)).toBe(2);
  expect(upcomingRadioPicks(origins, 0)).toBe(2);
  expect(radioSectionStart(origins, 2)).toBe(3);
  expect(upcomingRadioPicks(origins, 3)).toBe(0);
  // A pick the listener moved above a queued song is part of "Up next".
  expect(radioSectionStart(["generated", "manual", "generated"], -1)).toBe(2);
  expect(radioSectionStart(["manual", "manual"], 0)).toBe(-1);
});

test("the coming-up notice only counts down into a Radio pick", () => {
  expect(
    radioComingUpIn({ currentTime: 185, duration: 200, nextIsRadioPick: true }),
  ).toBe(15);
  expect(
    radioComingUpIn({ currentTime: 100, duration: 200, nextIsRadioPick: true }),
  ).toBeNull();
  expect(
    radioComingUpIn({
      currentTime: 185,
      duration: 200,
      nextIsRadioPick: false,
    }),
  ).toBeNull();
  expect(
    radioComingUpIn({
      currentTime: 185,
      duration: 200,
      nextIsRadioPick: true,
      looping: true,
    }),
  ).toBeNull();
  // Very short tracks would show the notice for their whole length.
  expect(
    radioComingUpIn({ currentTime: 1, duration: 20, nextIsRadioPick: true }),
  ).toBeNull();
  expect(
    radioComingUpIn({ currentTime: 0, duration: NaN, nextIsRadioPick: true }),
  ).toBeNull();
});

test("skip reasons map to distinct recommendation signals", () => {
  expect(radioSkipReasons.map((reason) => reason.event)).toEqual([
    "disliked",
    "skip_not_now",
    "skip_doesnt_fit",
  ]);
  expect(radioSkipReasonEvent("not_now")).toBe("skip_not_now");
});

test("Radio respects its switch and repeat modes", () => {
  expect(shouldContinueWithRadio({ enabled: true, repeating: false })).toBe(
    true,
  );
  expect(shouldContinueWithRadio({ enabled: false, repeating: false })).toBe(
    false,
  );
  expect(shouldContinueWithRadio({ enabled: true, repeating: true })).toBe(
    false,
  );
});

test("station labels name the seed", () => {
  expect(radioStationLabel(null)).toBe("Based on your queue");
  expect(radioStationLabel({ kind: "artist", name: "Nina Simone" })).toBe(
    "Playing Nina Simone radio",
  );
  expect(radioStationLabel({ kind: "song", name: "Blue" })).toBe(
    "Based on “Blue”",
  );
  expect(radioSeedKey({ kind: "genre", id: "Jazz" })).toBe("genre:Jazz");
  expect(radioSeedKey(null)).toBe("");
});

test("the queue becomes Radio's play history, most recent position wins", () => {
  expect(radioQueueHistory(["a", "b", "a", "", null, "c"])).toEqual([
    "b",
    "a",
    "c",
  ]);
  expect(radioQueueHistory(["a", "b", "c", "d"], 2)).toEqual(["c", "d"]);
  expect(radioQueueHistory([])).toEqual([]);
});
