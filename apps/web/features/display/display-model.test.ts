import { describe, expect, test } from "bun:test";
import type { DisplayState } from "@parson/music-sdk";
import {
  clockText,
  displayMode,
  lyricsWindow,
  pairingCountdown,
  suggestDisplayName,
  waveformBarCount,
} from "./display-model";

function state(overrides: Partial<DisplayState> = {}): DisplayState {
  return {
    display: {
      id: "d1",
      name: "TV",
      zone_id: null,
      lyrics: "auto",
      created_at: "",
      last_seen_at: null,
      online: true,
    },
    server_time_ms: 0,
    status: "playing",
    zone: null,
    track: {
      song_id: "s",
      title: "T",
      artist: "A",
      artist_id: null,
      album: "",
      album_id: null,
      year: null,
      composer: null,
      track_number: null,
      track_count: null,
      duration_ms: 1,
      format: null,
      cover_url: null,
      artist_image_url: null,
      artist_image_credit: null,
      biography: null,
      live: false,
    },
    up_next: [],
    ...overrides,
  };
}

const sung = [
  { time: 1, text: "Hold on" },
  { time: 4, text: "" },
];

describe("display mode", () => {
  test("pairs first, then waits for the server", () => {
    expect(
      displayMode({
        hasKey: false,
        state: null,
        timedLines: [],
        lyricsHidden: false,
      }),
    ).toBe("pairing");
    expect(
      displayMode({
        hasKey: true,
        state: null,
        timedLines: [],
        lyricsHidden: false,
      }),
    ).toBe("connecting");
  });

  test("timed lyrics take over unless switched off", () => {
    const base = { hasKey: true, timedLines: sung, lyricsHidden: false };
    expect(displayMode({ ...base, state: state() })).toBe("lyrics");
    expect(displayMode({ ...base, lyricsHidden: true, state: state() })).toBe(
      "info",
    );
    const off = state();
    off.display.lyrics = "off";
    expect(displayMode({ ...base, state: off })).toBe("info");
    // Only instrumental breaks: nothing to read.
    expect(
      displayMode({
        ...base,
        timedLines: [{ time: 0, text: " " }],
        state: state(),
      }),
    ).toBe("info");
    const live = state();
    live.track!.live = true;
    expect(displayMode({ ...base, state: live })).toBe("info");
  });

  test("an empty or missing zone shows the clock or says so", () => {
    const base = { hasKey: true, timedLines: [], lyricsHidden: false };
    expect(
      displayMode({
        ...base,
        state: state({ status: "nothing", track: null }),
      }),
    ).toBe("idle");
    expect(
      displayMode({
        ...base,
        state: state({ status: "zone_missing", track: null }),
      }),
    ).toBe("missing");
  });
});

test("helpers for the clock, the code and the seek bar", () => {
  expect(clockText(new Date(2026, 9, 27, 21, 7), "en-GB")).toBe("21:07");
  expect(pairingCountdown(10_000 + 581_000, 10_000)).toBe("9:41");
  expect(pairingCountdown(0, 5_000)).toBe("0:00");
  expect(waveformBarCount(1_800)).toBe(180);
  expect(waveformBarCount(100)).toBe(48);
  expect(waveformBarCount(9_000)).toBe(220);
  expect(waveformBarCount(Number.NaN)).toBe(120);
});

test("screens get a sensible first name", () => {
  expect(
    suggestDisplayName(
      "Mozilla/5.0 (Linux; Tizen 6.0; SMART-TV) AppleWebKit/537.36",
    ),
  ).toBe("TV");
  expect(suggestDisplayName("Mozilla/5.0 (Linux; Android 9; AFTMM) Silk")).toBe(
    "TV",
  );
  expect(suggestDisplayName("Mozilla/5.0 (X11; Linux) CrKey/1.56")).toBe(
    "Chromecast",
  );
  expect(
    suggestDisplayName("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)"),
  ).toBe("Tablet");
  expect(
    suggestDisplayName("Mozilla/5.0 (Linux; Android 14; Pixel Tablet)"),
  ).toBe("Tablet");
  expect(
    suggestDisplayName("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Mobile"),
  ).toBe("Phone");
  expect(suggestDisplayName("Mozilla/5.0 (Windows NT 10.0) Chrome/130")).toBe(
    "Browser",
  );
});

test("only lines near the sung one are rendered", () => {
  expect(lyricsWindow(0, -1)).toEqual({ start: 0, end: 0 });
  expect(lyricsWindow(100, -1)).toEqual({ start: 0, end: 8 });
  expect(lyricsWindow(100, 50)).toEqual({ start: 47, end: 58 });
  expect(lyricsWindow(10, 9)).toEqual({ start: 6, end: 10 });
});
