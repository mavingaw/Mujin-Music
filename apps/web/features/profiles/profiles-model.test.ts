import { expect, test } from "bun:test";
import type { Profile } from "@parson/music-sdk";
import { profileBadge, profileSubtitle, switchHint } from "./profiles-model";

const profile = (overrides: Partial<Profile>): Profile => ({
  id: 1,
  name: "Ada",
  username: "ada",
  kind: "account",
  role: "user",
  color: "violet",
  hasPin: false,
  quickSwitch: true,
  isCurrent: false,
  canEdit: false,
  canDelete: false,
  unlock: "none",
  listens: 0,
  scrobbling: [],
  ...overrides,
});

test("subtitles summarise plays, scrobbling and locks", () => {
  expect(profileSubtitle(profile({}))).toBe("New listener");
  expect(profileSubtitle(profile({ listens: 1 }))).toBe("1 play");
  expect(
    profileSubtitle(
      profile({
        listens: 1284,
        scrobbling: ["lastfm", "listenbrainz"],
        unlock: "pin",
      }),
    ),
  ).toBe("1,284 plays · Last.fm · ListenBrainz · PIN");
  expect(profileSubtitle(profile({ unlock: "password" }))).toBe(
    "New listener · Password",
  );
});

test("badges and hints", () => {
  expect(profileBadge(profile({ role: "admin" }))).toBe("Admin");
  expect(profileBadge(profile({ kind: "profile" }))).toBe("Profile");
  expect(profileBadge(profile({}))).toBeNull();
  expect(switchHint(profile({ isCurrent: true }))).toBe("Listening now");
  expect(switchHint(profile({ unlock: "pin" }))).toBe("Enter PIN to switch");
  expect(switchHint(profile({}))).toBe("Switch instantly");
});
