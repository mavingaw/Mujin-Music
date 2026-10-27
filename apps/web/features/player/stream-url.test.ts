import { describe, expect, test } from "bun:test";
import { playIdFromStreamUrl } from "@parson/music-sdk";
import {
  buildStreamUrl,
  nextPlayId,
  startedStream,
} from "@/lib/api/stream-url";

describe("media stream URLs", () => {
  test("use a media-scoped token without exposing the access token", () => {
    const url = new URL(
      buildStreamUrl(
        "https://music.example",
        "folder/song #1",
        320,
        true,
        "short-lived-media-token",
      ),
    );

    expect(url.pathname).toBe(
      "/api/v1/media/songs/folder%2Fsong%20%231/stream",
    );
    expect(url.searchParams.get("bitrate")).toBe("320");
    expect(url.searchParams.get("slowed_reverb")).toBe("true");
    expect(url.searchParams.get("media_token")).toBe("short-lived-media-token");
    expect(url.searchParams.has("access_token")).toBe(false);
  });

  test("same-origin cookie playback remains available without a media token", () => {
    const url = new URL(
      buildStreamUrl("http://localhost:1993", "song-1", 0, false),
    );

    expect(url.searchParams.has("media_token")).toBe(false);
    expect(url.searchParams.has("access_token")).toBe(false);
  });

  test("streaming plays carry an id that a preload and its play share", () => {
    const preload = nextPlayId("tidal:300101", 0);
    // The play that promotes the preload asks again and gets the same id.
    expect(nextPlayId("tidal:300101", 0)).toBe(preload);
    const url = buildStreamUrl(
      "https://music.example",
      "tidal:300101",
      0,
      false,
      null,
      preload,
    );
    expect(playIdFromStreamUrl(url)).toBe(preload);
    startedStream(url);
    // A replay (or the same track at another quality) is a new play.
    expect(nextPlayId("tidal:300101", 0)).not.toBe(preload);
    expect(nextPlayId("tidal:300101", 320)).not.toBe(preload);
  });

  test("library songs carry no play id", () => {
    const url = new URL(buildStreamUrl("https://music.example", "song-1"));
    expect(url.searchParams.has("play")).toBe(false);
  });
});
