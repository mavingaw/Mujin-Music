import { describe, expect, test } from "bun:test";
import {
  publishAppearanceChange,
  shouldFetchAnnounced,
  subscribeAppearanceChanges,
} from "./appearance-events";

describe("appearance events", () => {
  test("listeners hear announced saves until they unsubscribe", () => {
    const heard: (number | null)[] = [];
    const stop = subscribeAppearanceChanges((revision) => heard.push(revision));
    publishAppearanceChange(4);
    publishAppearanceChange(undefined);
    publishAppearanceChange(Number.NaN);
    stop();
    publishAppearanceChange(5);
    expect(heard).toEqual([4, null, null]);
  });

  test("a session fetches only saves it does not already show", () => {
    expect(shouldFetchAnnounced(4, 3, false)).toBeTrue();
    // Its own save comes back with the revision it already shows.
    expect(shouldFetchAnnounced(4, 4, false)).toBeFalse();
    // A lagged socket announces without a revision: always re-read.
    expect(shouldFetchAnnounced(null, 4, false)).toBeTrue();
    // Mid-edit, the local save wins and merges on top.
    expect(shouldFetchAnnounced(9, 4, true)).toBeFalse();
  });
});
