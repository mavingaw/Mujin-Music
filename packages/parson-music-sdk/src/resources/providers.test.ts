import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  addToStreamingLibrary,
  getStreamingAlbum,
  getStreamingLibrary,
  isStreamingId,
  parseStreamingId,
  searchStreaming,
  streamingAlbumToAlbum,
  streamingTrackToSong,
  type StreamingAlbum,
} from "./providers";

afterEach(() => {
  mock.restore();
});

const album: StreamingAlbum = {
  id: "qobuz:q0glassorch01",
  provider: "qobuz",
  title: "Glass Orchard",
  version: "Deluxe",
  artist: { id: "qobuz:51001", name: "Ilse Marten" },
  artists: [{ id: "qobuz:51001", name: "Ilse Marten" }],
  cover_url: "https://static.qobuz.test/cover.jpg",
  release_date: "2025-02-14",
  track_count: 1,
  duration: 274,
  explicit: false,
  format: {
    codec: "flac",
    lossless: true,
    bit_depth: 24,
    sample_rate: 192000,
    bitrate_kbps: null,
    label: "24/192 FLAC",
  },
  label: "Nordlicht Records",
  genre: "Classical",
  copyright: null,
  web_url: null,
  in_library: false,
  tracks: [
    {
      id: "qobuz:7700001",
      provider: "qobuz",
      title: "I. Bloom",
      version: null,
      artist: { id: "qobuz:51001", name: "Ilse Marten" },
      artists: [
        { id: "qobuz:51001", name: "Ilse Marten" },
        { id: "qobuz:9", name: "Guest Cellist" },
      ],
      album: null,
      track_number: 1,
      disc_number: 1,
      duration: 274,
      explicit: false,
      isrc: "FRQBZ7700001",
      format: {
        codec: "flac",
        lossless: true,
        bit_depth: 24,
        sample_rate: 192000,
        bitrate_kbps: null,
        label: "24/192 FLAC",
      },
      streamable: true,
      in_library: false,
    },
  ],
};

test("streaming ids are recognized without colliding with library ids", () => {
  expect(parseStreamingId("tidal:77646169")).toEqual({
    provider: "tidal",
    rawId: "77646169",
  });
  expect(isStreamingId("qobuz:0060254735180")).toBeTrue();
  for (const id of [
    "9f0c2d1e-0a7b-4c55-8f60-1b2c3d4e5f60",
    "spotify:1",
    "tidal:../x",
    "",
    null,
  ]) {
    expect(isStreamingId(id)).toBeFalse();
  }
});

test("streaming albums become playable library-shaped albums", () => {
  const converted = streamingAlbumToAlbum(album);
  expect(converted.name).toBe("Glass Orchard (Deluxe)");
  expect(converted.artist_object.name).toBe("Ilse Marten");
  const song = converted.songs[0];
  expect(song.id).toBe("qobuz:7700001");
  expect(song.album_object.id).toBe("qobuz:q0glassorch01");
  expect(song.album_object.cover_url).toBe(
    "https://static.qobuz.test/cover.jpg",
  );
  expect(song.contributing_artists).toEqual(["Guest Cellist"]);
  expect(song.audio).toMatchObject({
    bit_depth: 24,
    sample_rate: 192000,
    lossless: true,
  });
  expect(streamingTrackToSong(album.tracks[0]).album_object.id).toBe("");
});

test("streaming resources call provider-scoped endpoints", async () => {
  const calls: unknown[][] = [];
  spyOn(api, "get").mockImplementation(async (...args: unknown[]) => {
    calls.push(args);
    return { data: {}, status: 200, headers: new Headers() } as never;
  });
  const put = spyOn(api, "put").mockResolvedValue({
    data: { in_library: true, service_updated: true },
    status: 200,
    headers: new Headers(),
  } as never);

  await getStreamingAlbum("tidal:300100");
  await searchStreaming("harbor", { providers: ["qobuz"], limit: 5 });
  await getStreamingLibrary({ provider: "qobuz", kind: "album", q: "" });
  await addToStreamingLibrary("album", "qobuz:q0harborhr04");

  expect(calls[0][0]).toBe("/providers/tidal/albums/300100");
  expect(calls[1]).toEqual([
    "/providers/search",
    {
      params: { q: "harbor", providers: "qobuz", limit: 5 },
      timeout: 25_000,
    },
  ]);
  expect(calls[2]).toEqual([
    "/providers/library",
    { params: { provider: "qobuz", kind: "album" } },
  ]);
  expect(put).toHaveBeenCalledWith(
    "/providers/qobuz/library/album/q0harborhr04",
  );
  await expect(getStreamingAlbum("local-album")).rejects.toThrow();
});
