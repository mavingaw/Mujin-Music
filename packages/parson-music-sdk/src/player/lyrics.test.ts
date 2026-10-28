import { describe, expect, test } from "bun:test";
import {
  activeLyricIndex,
  lyricLineEmphasis,
  lyricsEmptyMessage,
  lyricsFailureKind,
  lyricsSourceLabel,
  parseLrc,
  parseLrcStamp,
} from "./lyrics";

describe("LRC parsing", () => {
  test("reads every common stamp variant", () => {
    expect(parseLrcStamp("01:02.5")).toBe(62.5);
    expect(parseLrcStamp("01:02.50")).toBe(62.5);
    expect(parseLrcStamp("01:02.503")).toBeCloseTo(62.503);
    expect(parseLrcStamp("01:02")).toBe(62);
    expect(parseLrcStamp("01:02:50")).toBe(62.5);
    expect(parseLrcStamp("1:02:03.25")).toBeCloseTo(3723.25);
    expect(parseLrcStamp("ar")).toBeNull();
    expect(parseLrcStamp("00:61.00")).toBeNull();
  });

  test("handles repeats, offsets, headers, word stamps and breaks", () => {
    const lines = parseLrc(
      "﻿[ar:Fixture Artist]\n[offset:+500]\r\n" +
        "[00:10.00][00:40.00]Chorus line\n" +
        "[00:05.00]<00:05.00>First <00:06.20>verse\n" +
        "[00:20.00]\nnot a lyric line",
    );
    expect(lines).toEqual([
      { time: 4.5, text: "First verse" },
      { time: 9.5, text: "Chorus line" },
      { time: 19.5, text: "" },
      { time: 39.5, text: "Chorus line" },
    ]);
    expect(parseLrc(null)).toEqual([]);
  });

  test("reads the server's normalised LRC", () => {
    expect(parseLrc("[00:01.00] One\n[00:02.50] Two")).toEqual([
      { time: 1, text: "One" },
      { time: 2.5, text: "Two" },
    ]);
  });
});

describe("the sung line", () => {
  const lines = parseLrc("[00:01.00]a\n[00:03.00]b\n[00:03.00]c\n[00:09.00]d");

  test("is the last line that has started", () => {
    expect(activeLyricIndex(lines, 0)).toBe(-1);
    expect(activeLyricIndex(lines, 1)).toBe(0);
    expect(activeLyricIndex(lines, 3.2)).toBe(2);
    expect(activeLyricIndex(lines, 600)).toBe(3);
    expect(activeLyricIndex([], 10)).toBe(-1);
  });

  test("is lit while following lines fade with distance", () => {
    expect(lyricLineEmphasis(4, 4)).toEqual({ state: "active", opacity: 1 });
    expect(lyricLineEmphasis(2, 4).state).toBe("past");
    const next = lyricLineEmphasis(5, 4);
    const later = lyricLineEmphasis(7, 4);
    const far = lyricLineEmphasis(40, 4);
    expect(next.state).toBe("next");
    expect(later.opacity).toBeLessThan(next.opacity);
    expect(far.opacity).toBeGreaterThan(0);
    expect(far.opacity).toBeLessThan(later.opacity);
    // Before the first line, the first line is "next".
    expect(lyricLineEmphasis(0, -1).state).toBe("next");
  });
});

describe("lyrics states", () => {
  const failure = (status: number, code?: string) => ({
    response: { status, data: code ? { code } : {} },
  });

  test("tell missing, offline, unavailable and failed apart", () => {
    expect(lyricsFailureKind(failure(404, "lyrics_not_found"))).toBe("missing");
    expect(lyricsFailureKind(failure(404, "lyrics_not_found_offline"))).toBe(
      "offline",
    );
    expect(lyricsFailureKind(failure(503, "lyrics_provider_unavailable"))).toBe(
      "unavailable",
    );
    expect(lyricsFailureKind(failure(503))).toBe("unavailable");
    expect(lyricsFailureKind(failure(500))).toBe("failed");
    expect(lyricsFailureKind(new Error("network"))).toBe("failed");
  });

  test("a 503 reads as a clear no-lyrics state with a retry", () => {
    const message = lyricsEmptyMessage("unavailable");
    expect(message.title).toBe("No lyrics for this track");
    expect(message.detail).toContain("503");
    expect(message.retry).toBe(true);
    expect(lyricsEmptyMessage("offline").detail).toContain("online metadata");
    expect(lyricsEmptyMessage(null, true).title).toBe("Instrumental");
  });

  test("name their source", () => {
    expect(lyricsSourceLabel("embedded_lrc")).toBe("LRC embedded in the file");
    expect(lyricsSourceLabel("lrclib")).toBe("LRCLIB");
    expect(lyricsSourceLabel(undefined)).toBeNull();
  });
});
