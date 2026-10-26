import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api, { ApiError } from "../core/http";
import {
  confidenceLabel,
  createImportJob,
  decodePlaylistFile,
  handOffToSpotify,
  importErrorMessage,
  importMatchRate,
  parseImportUrl,
  saveImportPlaylist,
  updateImportItems,
} from "./imports";

afterEach(() => mock.restore());

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("playlist links are recognised per service", () => {
  expect(
    parseImportUrl("https://open.spotify.com/playlist/37i9dQZF1DX?si=1"),
  ).toEqual({ service: "spotify", kind: "playlist", id: "37i9dQZF1DX" });
  expect(parseImportUrl("spotify:playlist:abc")).toEqual({
    service: "spotify",
    kind: "playlist",
    id: "abc",
  });
  expect(parseImportUrl("open.spotify.com/collection/tracks")?.kind).toBe(
    "saved_tracks",
  );
  expect(parseImportUrl("https://www.deezer.com/fr/playlist/123")).toEqual({
    service: "deezer",
    kind: "playlist",
    id: "123",
  });
  expect(
    parseImportUrl("https://music.youtube.com/playlist?list=PLabc_-1"),
  ).toEqual({ service: "youtube_music", kind: "playlist", id: "PLabc_-1" });
  expect(
    parseImportUrl("https://music.apple.com/us/playlist/hits/pl.f4d1"),
  ).toEqual({ service: "apple_music", kind: "playlist", id: "pl.f4d1" });
  expect(
    parseImportUrl("https://music.apple.com/library/playlist/p.abc")?.kind,
  ).toBe("library_playlist");
});

test("private lists and unrelated links are rejected", () => {
  for (const input of [
    "",
    "hello",
    "https://music.youtube.com/playlist?list=LM",
    "https://open.spotify.com/track/abc",
    "https://example.com/playlist/1",
    "spotify:playlist:a/b",
    "https://open.spotify.com/playlist/..",
  ]) {
    expect(parseImportUrl(input)).toBeNull();
  }
});

test("UTF-16 Apple Music exports and UTF-8 BOM files decode", () => {
  const utf16 = new Uint8Array([0xff, 0xfe, 0x4e, 0x00, 0x61, 0x00]);
  expect(decodePlaylistFile(utf16)).toBe("Na");
  const utf8 = new TextEncoder().encode("﻿#EXTM3U");
  expect(decodePlaylistFile(utf8)).toBe("#EXTM3U");
});

test("jobs, review actions and saving use the imports endpoints", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok({ id: "job" }));
  await createImportJob({ url: "https://www.deezer.com/playlist/1" });
  expect(post).toHaveBeenCalledWith(
    "/imports/jobs",
    { url: "https://www.deezer.com/playlist/1" },
    { timeout: 30_000 },
  );
  await saveImportPlaylist("job/1", "  ");
  expect(post).toHaveBeenLastCalledWith("/imports/jobs/job%2F1/playlist", {
    name: undefined,
  });

  const patch = spyOn(api, "patch").mockResolvedValue(ok({ id: "job" }));
  await updateImportItems("job", [
    { action: "choose", position: 2, song_id: "s1" },
  ]);
  expect(patch).toHaveBeenCalledWith("/imports/jobs/job/items", {
    actions: [{ action: "choose", position: 2, song_id: "s1" }],
  });
});

test("Spotify handoff is bounded, deduplicated and never asks for audio", async () => {
  const post = spyOn(api, "post").mockResolvedValue(
    ok({ started: true, resolved: 1, unresolved_song_ids: [] }),
  );
  const ids = Array.from({ length: 80 }, (_, index) => `s${index % 60}`);
  await handOffToSpotify(ids, "device", 1234.6);
  const [path, body] = post.mock.calls[0]!;
  expect(path).toBe("/imports/services/spotify/handoff");
  expect((body as { song_ids: string[] }).song_ids).toHaveLength(50);
  expect((body as { position_ms: number }).position_ms).toBe(1235);
});

test("backend messages surface and match rates ignore skipped tracks", () => {
  const error = new ApiError(
    "failed",
    {},
    {
      data: { message: "Connect Spotify first." },
      status: 409,
      headers: new Headers(),
    },
  );
  expect(importErrorMessage(error, "fallback")).toBe("Connect Spotify first.");
  expect(importErrorMessage(new Error("x"), "fallback")).toBe("fallback");
  expect(
    importMatchRate({ total: 10, skipped: 2, matched: 6 } as never),
  ).toBeCloseTo(0.75);
  expect(confidenceLabel(0.9)).toBe("high");
  expect(confidenceLabel(0.6)).toBe("medium");
  expect(confidenceLabel(0.2)).toBe("low");
});
