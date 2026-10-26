import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
import api, { ApiError } from "../core/http";
import {
  accountProof,
  albumPlayChoice,
  matchAlbumVersions,
  streamingSignInPath,
  tidalSignInMethods,
  type StreamingProviderStatus,
  type StreamingAccountCheck,
  finishTidalBrowserSignIn,
  readTidalCallback,
  startTidalBrowserSignIn,
  tidalRedirectUri,
  createStreamingPlaylistCopy,
  creditLines,
  describeRadio,
  editStreamingPlaylistCopy,
  getStreamingRadio,
  moveTrackId,
  prettyRole,
  addToStreamingLibrary,
  formatScore,
  getSourcePreference,
  localAlbumFormat,
  rankAlbumVersions,
  setSourcePreference,
  type AlbumVersion,
  checkStreamingConnection,
  describeDelivery,
  getStreamDelivery,
  isPlayId,
  newPlayId,
  playIdFromStreamUrl,
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
  getCatalogAlbums,
  getStreamingGenreAlbums,
  getStreamingGenres,
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

test("genres, genre pages and the unified album catalog", async () => {
  const calls: unknown[][] = [];
  spyOn(api, "get").mockImplementation(async (...args: unknown[]) => {
    calls.push(args);
    return {
      data: { genres: [{ id: "Jazz", provider: "tidal", name: "Jazz" }] },
      status: 200,
      headers: new Headers(),
    } as never;
  });

  expect(await getStreamingGenres("tidal")).toEqual([
    { id: "Jazz", provider: "tidal", name: "Jazz" },
  ]);
  await getStreamingGenreAlbums("qobuz", "112", {
    section: "editors_picks",
    limit: 24,
  });
  await getCatalogAlbums(
    { source: "qobuz", genre: "Pop/Rock", decade: 1990, q: "" },
    { offset: 100, limit: 100 },
  );
  await getCatalogAlbums();

  expect(calls[0][0]).toBe("/providers/tidal/genres");
  expect(calls[1]).toEqual([
    "/providers/qobuz/genres/112/albums",
    { params: { section: "editors_picks", limit: 24 } },
  ]);
  expect(calls[2]).toEqual([
    "/providers/catalog/albums",
    {
      params: {
        source: "qobuz",
        genre: "Pop/Rock",
        decade: 1990,
        offset: 100,
        limit: 100,
      },
    },
  ]);
  expect(calls[3]).toEqual(["/providers/catalog/albums", { params: {} }]);
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

  // One play's report, so a replay at another quality never shows the old one.
  get.mockResolvedValueOnce({
    data: { ...delivery, play: "pabc123" },
    status: 200,
    headers: new Headers(),
  } as never);
  expect((await getStreamDelivery("qobuz:7700001", "pabc123"))?.play).toBe(
    "pabc123",
  );
  expect(get).toHaveBeenLastCalledWith(
    "/providers/qobuz/tracks/7700001/delivery",
    {
      params: { play: "pabc123" },
    },
  );

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

test("play ids are fresh, URL-safe and read back from stream URLs", () => {
  const first = newPlayId();
  const second = newPlayId();
  expect(first).not.toBe(second);
  expect(isPlayId(first)).toBeTrue();
  expect(isPlayId("has space")).toBeFalse();
  expect(isPlayId("x".repeat(65))).toBeFalse();
  expect(
    playIdFromStreamUrl(
      `https://parson.test/api/v1/media/songs/tidal%3A1/stream?bitrate=0&play=${first}&media_token=t`,
    ),
  ).toBe(first);
  expect(
    playIdFromStreamUrl("https://parson.test/stream?bitrate=0"),
  ).toBeNull();
  expect(playIdFromStreamUrl("https://x.test/s?play=bad%20id")).toBeNull();
  expect(playIdFromStreamUrl(null)).toBeNull();
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

describe("album versions", () => {
  const hiRes: AlbumVersion = {
    id: "qobuz:q1",
    title: "Glass Orchard",
    source: "qobuz",
    format: {
      lossless: true,
      bit_depth: 24,
      sample_rate: 192_000,
      bitrate_kbps: null,
      label: "24/192 FLAC",
    },
  };
  const max96: AlbumVersion = {
    id: "tidal:t1",
    title: "Glass Orchard",
    source: "tidal",
    format: {
      lossless: true,
      bit_depth: 24,
      sample_rate: 96_000,
      bitrate_kbps: null,
      label: "24/96 FLAC",
    },
  };
  const cd: AlbumVersion = {
    id: "local-1",
    title: "Glass Orchard",
    source: "local",
    format: {
      lossless: true,
      bit_depth: 16,
      sample_rate: 44_100,
      bitrate_kbps: null,
      label: "16/44.1 FLAC",
    },
  };

  test("highest quality plays the best-sounding copy", () => {
    const ranked = rankAlbumVersions([cd, max96, hiRes], "highest_quality");
    expect(ranked.map((version) => version.id)).toEqual([
      "qobuz:q1",
      "tidal:t1",
      "local-1",
    ]);
    expect(ranked[0]?.preferred).toBe(true);
    expect(ranked[0]?.reason).toBe("Highest quality: 24/192 FLAC");
    expect(ranked.slice(1).every((version) => !version.preferred)).toBe(true);
  });

  test("a favored source wins even at lower quality", () => {
    const ranked = rankAlbumVersions([hiRes, cd, max96], "local");
    expect(ranked[0]?.id).toBe("local-1");
    expect(ranked[0]?.reason).toBe("My files first in your settings");
    expect(ranked[1]?.id).toBe("qobuz:q1");
    const missing = rankAlbumVersions([cd, max96], "qobuz");
    expect(missing[0]?.id).toBe("tidal:t1");
    expect(missing[0]?.reason).toBe("Not in Qobuz; best of the rest");
  });

  test("ties go to local files, then Qobuz", () => {
    const tidalCd = { ...cd, id: "tidal:t2", source: "tidal" as const };
    const qobuzCd = { ...cd, id: "qobuz:q2", source: "qobuz" as const };
    const ranked = rankAlbumVersions([tidalCd, qobuzCd, cd], "highest_quality");
    expect(ranked.map((version) => version.id)).toEqual([
      "local-1",
      "qobuz:q2",
      "tidal:t2",
    ]);
    expect(ranked[0]?.reason).toBe(
      "Same quality; local files and Qobuz win ties",
    );
  });

  test("lossless beats any lossy bitrate and unknown formats sort last", () => {
    expect(
      formatScore({
        lossless: false,
        bit_depth: null,
        sample_rate: null,
        bitrate_kbps: 320,
        label: "AAC 320",
      }),
    ).toBeLessThan(formatScore(cd.format));
    expect(formatScore(null)).toBeLessThan(0);
  });

  test("local album format is the best file's technical properties", () => {
    expect(
      localAlbumFormat([
        { audio: null },
        {
          audio: {
            codec: "flac",
            container: "flac",
            sample_rate: 44_100,
            bit_depth: 16,
            channels: 2,
            bitrate_kbps: 900,
            lossless: true,
          },
        },
        {
          audio: {
            codec: "flac",
            container: "flac",
            sample_rate: 88_200,
            bit_depth: 24,
            channels: 2,
            bitrate_kbps: 3000,
            lossless: true,
          },
        },
      ])?.label,
    ).toBe("24/88.2 FLAC");
    expect(localAlbumFormat([])).toBeNull();
  });

  test("source preference reads and writes the server setting", async () => {
    const get = spyOn(api, "get").mockResolvedValue({
      data: { source: "tidal" },
    } as never);
    expect(await getSourcePreference()).toBe("tidal");
    expect(get).toHaveBeenCalledWith("/providers/preferences/source");
    const put = spyOn(api, "put").mockResolvedValue({
      data: { source: "local" },
    } as never);
    expect(await setSourcePreference("local")).toBe("local");
    expect(put).toHaveBeenCalledWith("/providers/preferences/source", {
      source: "local",
    });
  });
});

describe("radio, credits and editable copies", () => {
  test("radio and copy routes use the provider-scoped paths", async () => {
    const get = spyOn(api, "get").mockResolvedValue({
      data: { tracks: [], origin: "similar_artists" },
      status: 200,
      headers: new Headers(),
    } as never);
    const patch = spyOn(api, "patch").mockResolvedValue({
      data: {},
      status: 200,
      headers: new Headers(),
    } as never);
    const post = spyOn(api, "post").mockResolvedValue({
      data: {},
      status: 200,
      headers: new Headers(),
    } as never);
    const put = spyOn(api, "put").mockResolvedValue({
      data: {},
      status: 200,
      headers: new Headers(),
    } as never);

    const radio = await getStreamingRadio("album", "qobuz:q0glassorch01");
    expect(radio.origin).toBe("similar_artists");
    expect(get.mock.calls[0]?.[0]).toBe(
      "/providers/qobuz/radio/album/q0glassorch01",
    );
    await expect(getStreamingRadio("track", "local-song")).rejects.toThrow();

    await editStreamingPlaylistCopy("tidal:7a3f", {
      title: "Mine",
      track_ids: ["tidal:2", "tidal:1"],
    });
    expect(patch.mock.calls[0]?.[0]).toBe(
      "/providers/tidal/playlists/7a3f/copy",
    );
    expect(patch.mock.calls[0]?.[1]).toEqual({
      title: "Mine",
      track_ids: ["tidal:2", "tidal:1"],
    });

    await createStreamingPlaylistCopy("qobuz", {
      title: "Radio",
      track_ids: ["qobuz:1"],
    });
    expect(post.mock.calls[0]?.[0]).toBe("/providers/qobuz/playlist-copies");

    await saveStreamingPlaylistCopy("tidal:7a3f", { replaceEdits: true });
    expect(put.mock.calls[0]?.[2]).toEqual({
      timeout: 30_000,
      params: { replace_edits: true },
    });
  });

  test("radio origins are named honestly", () => {
    expect(describeRadio("tidal", { origin: "service" })).toBe("TIDAL radio");
    expect(describeRadio("qobuz", { origin: "similar_artists" })).toBe(
      "Parson radio from Qobuz similar artists",
    );
  });

  test("tracks move within the bounds of a copy", () => {
    expect(moveTrackId(["a", "b", "c"], 2, -1)).toEqual(["a", "c", "b"]);
    expect(moveTrackId(["a", "b", "c"], 0, 1)).toEqual(["b", "a", "c"]);
    expect(moveTrackId(["a", "b"], 0, -1)).toEqual(["a", "b"]);
    expect(moveTrackId(["a", "b"], 1, 1)).toEqual(["a", "b"]);
  });

  test("credit lines put performers and writers first", () => {
    expect(
      creditLines([
        { role: "Producer", names: ["Ines Calder"] },
        { role: "Composer", names: ["Sora Ueda"] },
        { role: "MainArtist", names: ["Sora Ueda"] },
        { role: "Mixing Engineer", names: [] },
      ]),
    ).toEqual([
      "Main artist: Sora Ueda",
      "Composer: Sora Ueda",
      "Producer: Ines Calder",
    ]);
    expect(creditLines(undefined)).toEqual([]);
    expect(prettyRole("Mastering Engineer")).toBe("Mastering engineer");
  });
});

describe("TIDAL browser sign-in", () => {
  test("posts the redirect URI, then the returned code and state", async () => {
    const post = spyOn(api, "post").mockResolvedValue({
      data: {},
      status: 200,
      headers: new Headers(),
    } as never);
    const redirect = tidalRedirectUri("https://music.example.test/");
    expect(redirect).toBe(
      "https://music.example.test/settings/services/tidal-callback",
    );
    await startTidalBrowserSignIn(redirect);
    await finishTidalBrowserSignIn("s1", "c1");
    expect(post.mock.calls[0]?.slice(0, 2)).toEqual([
      "/providers/tidal/sign-in/browser",
      { redirect_uri: redirect },
    ]);
    expect(post.mock.calls[1]?.slice(0, 2)).toEqual([
      "/providers/tidal/sign-in/browser/finish",
      { state: "s1", code: "c1" },
    ]);
  });

  test("reads approvals, refusals and empty callbacks", () => {
    expect(readTidalCallback("?code=abc&state=xyz")).toEqual({
      ok: true,
      code: "abc",
      state: "xyz",
    });
    expect(readTidalCallback("?error=access_denied&state=xyz")).toEqual({
      ok: false,
      error: "Sign-in was declined on TIDAL.",
    });
    expect(
      readTidalCallback("?error=invalid_scope&error_description=Bad+scope"),
    ).toEqual({ ok: false, error: "Bad scope" });
    expect(readTidalCallback("")).toEqual({
      ok: false,
      error: "TIDAL sent back no sign-in code.",
    });
  });
});

describe("account proof", () => {
  const now = new Date("2026-10-26T12:00:00Z");
  const base: StreamingAccountCheck = {
    checked_at: Math.floor(now.getTime() / 1000) - 60,
    ok: true,
    sandbox: false,
    verified: true,
    failed_step: null,
    detail: "Delivered 24/192 FLAC",
    requested: "hi_res",
    announced: "24/192 FLAC",
    delivered: "24/192 FLAC",
    lossless: true,
    matches_announced: true,
    full_length: true,
    delivered_seconds: 274,
    track_seconds: 274,
    track_title: "Blue in Green",
  };
  const provider = (last_check: StreamingAccountCheck | null) => ({
    name: "Qobuz",
    status: "signed_in" as const,
    last_check,
  });

  test("a signed-in account is never assumed to work", () => {
    expect(accountProof(provider(null), now)?.tone).toBe("unproven");
    expect(
      accountProof({ name: "Qobuz", status: "configured", last_check: null }),
    ).toBeNull();
  });

  test("only a real full-length lossless check is verified", () => {
    const proof = accountProof(provider(base), now);
    expect(proof?.tone).toBe("verified");
    expect(proof?.label).toBe("Verified: full-length 24/192 FLAC");
    expect(proof?.detail).toContain('"Blue in Green" as 24/192 FLAC');

    const sandbox = accountProof(
      provider({ ...base, sandbox: true, verified: false }),
      now,
    );
    expect(sandbox?.tone).toBe("unproven");
    expect(sandbox?.detail).toContain("not the real service");
  });

  test("lossy and preview results say what was not proven", () => {
    const lossy = accountProof(
      provider({
        ...base,
        verified: false,
        lossless: false,
        delivered: "AAC 320",
      }),
      now,
    );
    expect(lossy?.tone).toBe("partial");
    expect(lossy?.detail).toContain("lossy (AAC 320)");

    const preview = accountProof(
      provider({
        ...base,
        ok: false,
        verified: false,
        failed_step: "stream",
        full_length: false,
        detail: "only 0:30 of 4:34 arrived, a preview",
      }),
      now,
    );
    expect(preview?.tone).toBe("failed");
    expect(preview?.label).toBe("Last check: Preview only");
    expect(preview?.detail).toContain("a preview");
  });
});

describe("sign-in with only a subscription", () => {
  const base: Pick<
    StreamingProviderStatus,
    | "credentials"
    | "server_credentials_available"
    | "server_credentials_source"
    | "key_service"
  > = {
    credentials: {
      configured: false,
      client_id_hint: null,
      has_secret: false,
      uses_server_credentials: false,
    },
    server_credentials_available: true,
    server_credentials_source: "key_service",
    key_service: {
      url: "https://keys.parson.test",
      issuer: "Parson",
      offers: true,
      device_code: false,
      error: null,
    },
  };

  test("a key service means the listener only signs in", () => {
    expect(streamingSignInPath(base)).toEqual({
      kind: "subscription",
      via: "key_service",
      issuer: "Parson",
    });
    expect(tidalSignInMethods(base)).toEqual({
      primary: "browser",
      secondary: null,
    });
    expect(
      tidalSignInMethods({
        ...base,
        key_service: { ...base.key_service!, device_code: true },
      }),
    ).toEqual({ primary: "device_code", secondary: "browser" });
  });

  test("owner keys, own keys and no keys are told apart", () => {
    expect(
      streamingSignInPath({ ...base, server_credentials_source: "settings" }),
    ).toEqual({ kind: "subscription", via: "server" });
    expect(
      streamingSignInPath({
        ...base,
        credentials: { ...base.credentials, configured: true },
      }),
    ).toEqual({ kind: "own_credentials" });
    expect(
      streamingSignInPath({
        ...base,
        server_credentials_available: false,
        server_credentials_source: null,
        key_service: { ...base.key_service!, offers: false, error: "timeout" },
      }),
    ).toEqual({ kind: "setup_needed", keyServiceError: "timeout" });
    expect(
      tidalSignInMethods({ ...base, server_credentials_source: "settings" }),
    ).toEqual({ primary: "device_code", secondary: "browser" });
  });
});

describe("album Play follows the source preference", () => {
  const lossless = (bit_depth: number, sample_rate: number, label: string) => ({
    lossless: true,
    bit_depth,
    sample_rate,
    bitrate_kbps: null,
    label,
  });
  const subject = {
    id: "qobuz:q1",
    title: "Kind of Blue (Legacy Edition)",
    artistName: "Miles Davis",
    source: "qobuz" as const,
    format: lossless(16, 44_100, "16/44.1 FLAC"),
  };

  test("matches copies by normalized title and artist, once each", () => {
    const versions = matchAlbumVersions(
      subject,
      [
        {
          id: "tidal:t1",
          title: "Kind Of Blue",
          provider: "tidal",
          artist: { id: "tidal:a", name: "Miles Davis", provider: "tidal" },
          format: lossless(24, 192_000, "24/192 FLAC"),
        },
        {
          id: "tidal:t2",
          title: "Kind of Blue",
          provider: "tidal",
          artist: { id: "tidal:b", name: "A Tribute Band", provider: "tidal" },
          format: null,
        },
        {
          id: "qobuz:q1",
          title: "Kind of Blue",
          provider: "qobuz",
          artist: { id: "qobuz:a", name: "Miles Davis", provider: "qobuz" },
          format: null,
        },
      ] as never,
      [
        {
          id: "42",
          title: "Kind of Blue",
          artistName: "Miles Davis",
          format: lossless(24, 96_000, "24/96 FLAC"),
        },
      ],
    );
    expect(versions.map((version) => version.id)).toEqual([
      "qobuz:q1",
      "tidal:t1",
      "42",
    ]);
  });

  test("Play starts the preferred copy, not the open page's", () => {
    const versions: AlbumVersion[] = [
      { id: "qobuz:q1", title: "x", source: "qobuz", format: subject.format },
      {
        id: "tidal:t1",
        title: "x",
        source: "tidal",
        format: lossless(24, 192_000, "24/192 FLAC"),
      },
      {
        id: "42",
        title: "x",
        source: "local",
        format: lossless(24, 96_000, "24/96 FLAC"),
      },
    ];
    const best = albumPlayChoice(
      "qobuz:q1",
      rankAlbumVersions(versions, "highest_quality"),
    );
    expect(best?.version.id).toBe("tidal:t1");
    expect(best?.isThisCopy).toBe(false);
    expect(best?.label).toBe("TIDAL · 24/192 FLAC");

    const tidalFirst = albumPlayChoice(
      "42",
      rankAlbumVersions(
        versions.map((version) =>
          version.source === "tidal" ? { ...version, format: null } : version,
        ),
        "tidal",
      ),
    );
    expect(tidalFirst?.version.id).toBe("tidal:t1");

    const mine = albumPlayChoice("42", rankAlbumVersions(versions, "local"));
    expect(mine?.isThisCopy).toBe(true);
    expect(mine?.label).toBe("Your files · 24/96 FLAC");

    expect(
      albumPlayChoice("42", rankAlbumVersions([versions[2]!], "tidal")),
    ).toBeNull();
  });
});
