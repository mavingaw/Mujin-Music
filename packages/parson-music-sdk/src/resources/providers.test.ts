import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api, { ApiError } from "../core/http";
import {
  addToStreamingLibrary,
  checkStreamingConnection,
  describeDelivery,
  getStreamDelivery,
  getStreamingPlaylist,
  getStreamingPlaylistCopies,
  getStreamingPlaylists,
  getStreamingServerCredentials,
  removeStreamingPlaylistCopy,
  saveStreamingPlaylistCopy,
  removeStreamingServerCredentials,
  saveStreamingServerCredentials,
  type StreamingDelivery,
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

test("server credentials and connection checks use their own routes", async () => {
  const get = spyOn(api, "get").mockResolvedValue({
    data: { providers: [] },
    status: 200,
    headers: new Headers(),
  } as never);
  const put = spyOn(api, "put").mockResolvedValue({
    data: { providers: [] },
    status: 200,
    headers: new Headers(),
  } as never);
  const del = spyOn(api, "delete").mockResolvedValue({
    data: { providers: [] },
    status: 200,
    headers: new Headers(),
  } as never);
  const post = spyOn(api, "post").mockResolvedValue({
    data: { ok: true, steps: [] },
    status: 200,
    headers: new Headers(),
  } as never);

  expect(await getStreamingServerCredentials()).toEqual([]);
  await saveStreamingServerCredentials("qobuz", {
    client_id: "100000001",
    client_secret: "secret",
  });
  await removeStreamingServerCredentials("tidal");
  await checkStreamingConnection("qobuz", "qobuz:7700001");
  await checkStreamingConnection("tidal");

  expect(get).toHaveBeenCalledWith("/providers/server-credentials");
  expect(put).toHaveBeenCalledWith("/providers/qobuz/server-credentials", {
    client_id: "100000001",
    client_secret: "secret",
  });
  expect(del).toHaveBeenCalledWith("/providers/tidal/server-credentials");
  expect(post.mock.calls[0]).toEqual([
    "/providers/qobuz/check",
    { track_id: "qobuz:7700001" },
    { timeout: 120_000 },
  ]);
  expect(post.mock.calls[1][1]).toEqual({});
});

const delivery: StreamingDelivery = {
  track_id: "qobuz:7700001",
  requested: "hi_res_max",
  announced: {
    codec: "flac",
    lossless: true,
    bit_depth: 24,
    sample_rate: 192000,
    bitrate_kbps: null,
    label: "24/192 FLAC",
  },
  delivered: {
    codec: "flac",
    lossless: true,
    bit_depth: 24,
    sample_rate: 192000,
    bitrate_kbps: null,
    channels: 2,
    label: "24/192 FLAC",
  },
  container: "flac",
  frame_verified: true,
  matches_announced: true,
  sandbox: false,
  at: 1792884382,
};

test("deliveries read back what the stream carried", async () => {
  const get = spyOn(api, "get").mockResolvedValueOnce({
    data: delivery,
    status: 200,
    headers: new Headers(),
  } as never);
  expect(await getStreamDelivery("qobuz:7700001")).toEqual(delivery);
  expect(get).toHaveBeenCalledWith("/providers/qobuz/tracks/7700001/delivery");

  get.mockRejectedValueOnce(
    new ApiError("Not found", {}, {
      data: {},
      status: 404,
      headers: new Headers(),
    } as never),
  );
  expect(await getStreamDelivery("qobuz:7700001")).toBeNull();
  get.mockRejectedValueOnce(
    new ApiError("Down", {}, {
      data: {},
      status: 502,
      headers: new Headers(),
    } as never),
  );
  await expect(getStreamDelivery("qobuz:7700001")).rejects.toThrow("Down");

  expect(describeDelivery(delivery)).toBe("24/192 FLAC, stereo");
  expect(
    describeDelivery({
      ...delivery,
      delivered: { ...delivery.announced, channels: 1 },
    }),
  ).toBe("24/192 FLAC, mono");
  expect(describeDelivery({ ...delivery, delivered: null })).toBe(
    "Format not readable",
  );
});

test("playlists are read and copied through provider-scoped routes", async () => {
  const get = spyOn(api, "get").mockResolvedValue({
    data: { playlists: [], copies: [], total: 0 },
    status: 200,
    headers: new Headers(),
  } as never);
  const put = spyOn(api, "put").mockResolvedValue({
    data: {},
    status: 200,
    headers: new Headers(),
  } as never);
  const del = spyOn(api, "delete").mockResolvedValue({
    data: undefined,
    status: 204,
    headers: new Headers(),
  } as never);
  const uuid = "7a3f2c10-5b4e-4d1a-9c2b-0e1f2a3b4c5d";

  await getStreamingPlaylists("tidal", { limit: 20 });
  await getStreamingPlaylist(`tidal:${uuid}`);
  expect(await getStreamingPlaylistCopies()).toEqual([]);
  await saveStreamingPlaylistCopy("qobuz:9100001");
  await removeStreamingPlaylistCopy("qobuz:9100001");

  expect(get.mock.calls[0]).toEqual([
    "/providers/tidal/playlists",
    { params: { limit: 20 } },
  ]);
  expect(get.mock.calls[1][0]).toBe(`/providers/tidal/playlists/${uuid}`);
  expect(get.mock.calls[2][0]).toBe("/providers/playlist-copies");
  expect(put.mock.calls[0][0]).toBe("/providers/qobuz/playlists/9100001/copy");
  expect(del).toHaveBeenCalledWith("/providers/qobuz/playlists/9100001/copy");
});
