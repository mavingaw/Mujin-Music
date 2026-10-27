import { expect, test } from "bun:test";
import {
  liveCountryName,
  liveFormatText,
  liveNowPlayingLine,
  liveStationSubtitle,
  liveStationTrack,
  liveTitleText,
  looksLikeStreamUrl,
  parseTagInput,
} from "./live-radio";

const station = {
  region: "Paris",
  country: "France",
  country_code: "FR",
  tags: [
    "aac",
    "pop/rock",
    "128k",
    "french",
    "a-very-long-tag-name-that-is-noise",
  ],
  source: "directory" as const,
};

test("subtitles show place and genres", () => {
  expect(liveStationSubtitle(station)).toBe(
    "Paris, France · Pop/Rock / French",
  );
  expect(
    liveStationSubtitle({
      region: null,
      country: "The United States Of America",
      country_code: null,
      tags: [],
      source: "directory",
    }),
  ).toBe("United States Of America");
  expect(
    liveStationSubtitle({
      region: null,
      country: null,
      country_code: null,
      tags: [],
      source: "custom",
    }),
  ).toBe("Added by URL");
  expect(liveCountryName("us")).toBe("United States");
  expect(liveCountryName("zz", null)).toBeNull();
});

test("now-playing lines prefer the current title", () => {
  const title = {
    raw: "Miles Davis - So What",
    artist: "Miles Davis",
    title: "So What",
    at: 0,
  };
  expect(liveTitleText(title)).toBe("Miles Davis — So What");
  expect(liveTitleText({ ...title, artist: null, title: "The news" })).toBe(
    "The news",
  );
  expect(liveNowPlayingLine(station, { title })).toBe("Miles Davis — So What");
  expect(liveNowPlayingLine(station, { title: null })).toBe(
    "Paris, France · Pop/Rock / French",
  );
});

test("formats read naturally and lossless hides bitrate", () => {
  expect(
    liveFormatText({
      codec: "aac",
      bitrate_kbps: 128,
      sample_rate: 48_000,
      bit_depth: null,
      channels: 2,
    }),
  ).toBe("AAC · 128 kbps · 48 kHz · stereo");
  expect(
    liveFormatText({
      codec: "flac",
      bitrate_kbps: 900,
      sample_rate: 44_100,
      bit_depth: 16,
      channels: 2,
    }),
  ).toBe("FLAC · 44.1 kHz · 16-bit · stereo");
});

test("stations become live player tracks", () => {
  const track = liveStationTrack(
    { ...station, song_id: "radio:abc", name: "Europe 2" },
    "https://x/logo",
  );
  expect(track.id).toBe("radio:abc");
  expect(track.duration).toBe(0);
  expect(track.album_object.cover_url).toBe("https://x/logo");
  expect(track.artist).toBe("Paris, France · Pop/Rock / French");
});

test("tag input and stream URL checks", () => {
  expect(parseTagInput("Jazz, jazz; Late Night / ,")).toEqual([
    "jazz",
    "late night",
  ]);
  expect(looksLikeStreamUrl("https://radio.example/live.pls")).toBeTrue();
  expect(looksLikeStreamUrl("radio.example/live")).toBeFalse();
  expect(looksLikeStreamUrl("https://user:pw@radio.example/")).toBeFalse();
  expect(looksLikeStreamUrl("ftp://radio.example/")).toBeFalse();
});
