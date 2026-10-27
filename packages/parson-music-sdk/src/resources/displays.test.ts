import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  DISPLAY_KEY_HEADER,
  approveDisplay,
  createDisplay,
  displayAssetURL,
  displayCaption,
  displayDrift,
  displayKeyFromHash,
  displayLink,
  displayPollDelay,
  displayQualityChip,
  displayServerOrigin,
  displayTrackLine,
  estimateDisplayPosition,
  formatDisplayTime,
  getDisplayLyrics,
  getDisplayState,
  getDisplayWaveform,
  isDisplayUnpaired,
  pollDisplayPairing,
  removeDisplay,
  startDisplayPairing,
  updateDisplay,
  waveformBars,
  type DisplayState,
  type DisplayTrack,
  type DisplayZone,
} from "./displays";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown, status = 200) =>
  ({ data, status, headers: new Headers() }) as never;

function track(overrides: Partial<DisplayTrack> = {}): DisplayTrack {
  return {
    song_id: "song-2",
    title: "Variatio 1",
    artist: "Kimiko Ishizaka",
    artist_id: "artist-1",
    album: "The Open Goldberg Variations",
    album_id: "album-1",
    year: "2012",
    composer: "Johann Sebastian Bach",
    track_number: 2,
    track_count: 32,
    duration_ms: 180_000,
    format: {
      label: "FLAC 96 kHz/24-bit",
      tier: "hi_res",
      codec: "flac",
      sample_rate: 96_000,
      bit_depth: 24,
      channels: 2,
    },
    cover_url: "/media/images/x?expires=1&image_signature=y",
    artist_image_url: null,
    artist_image_credit: null,
    biography: null,
    live: false,
    ...overrides,
  };
}

function zone(overrides: Partial<DisplayZone> = {}): DisplayZone {
  return {
    id: "client:living",
    name: "Living room",
    icon: "monitor",
    kind: "web",
    status: "playing",
    position_ms: 60_000,
    duration_ms: 180_000,
    shuffle: false,
    repeat: "off",
    quality: null,
    bit_perfect: null,
    output: null,
    steps: [],
    ...overrides,
  };
}

function state(overrides: Partial<DisplayState> = {}): DisplayState {
  return {
    display: {
      id: "d1",
      name: "TV",
      zone_id: null,
      lyrics: "auto",
      created_at: "2026-10-27 10:00:00",
      last_seen_at: null,
      online: true,
    },
    server_time_ms: 0,
    status: "playing",
    zone: zone(),
    track: track(),
    up_next: [],
    ...overrides,
  };
}

test("screen calls carry only the display key and never the session", async () => {
  const get = spyOn(api, "get").mockResolvedValue(ok(state()));
  await getDisplayState("mjd_abc");
  expect(get).toHaveBeenCalledWith("/display/state", {
    anonymous: true,
    timeout: 8_000,
    headers: { [DISPLAY_KEY_HEADER]: "mjd_abc" },
  });

  get.mockResolvedValue(ok({ syncedLyrics: "[00:01.00]Hi" }));
  await getDisplayLyrics("mjd_abc", "a/b");
  expect(get).toHaveBeenLastCalledWith("/display/lyrics/a%2Fb", {
    anonymous: true,
    timeout: 30_000,
    headers: { [DISPLAY_KEY_HEADER]: "mjd_abc" },
  });

  get.mockResolvedValue(ok({ status: "analysing" }, 202));
  expect(await getDisplayWaveform("mjd_abc", "s1")).toBeNull();
  get.mockResolvedValue(ok({ status: "ready", peaks: [1, 2], rms: [0, 1] }));
  expect(await getDisplayWaveform("mjd_abc", "s1")).toEqual({
    peaks: [1, 2],
    rms: [0, 1],
  });
});

test("pairing starts anonymously and an expired code reads as expired", async () => {
  const post = spyOn(api, "post").mockResolvedValue(
    ok({ pairing_id: "p", secret: "s", code: "123456", expires_in: 600 }),
  );
  expect((await startDisplayPairing("Bedroom TV")).code).toBe("123456");
  expect(post).toHaveBeenCalledWith(
    "/display/pair",
    { name: "Bedroom TV" },
    { anonymous: true, timeout: 8_000 },
  );

  post.mockRejectedValue({ response: { status: 404 } });
  expect(await pollDisplayPairing({ pairing_id: "p", secret: "s" })).toEqual({
    status: "expired",
  });
  post.mockRejectedValue({ response: { status: 500 } });
  await expect(
    pollDisplayPairing({ pairing_id: "p", secret: "s" }),
  ).rejects.toBeDefined();
});

test("management calls use the signed-in session", async () => {
  const post = spyOn(api, "post").mockResolvedValue(
    ok({ display: { id: "d1" }, key: "mjd_k" }),
  );
  expect((await createDisplay({ name: "TV", zone_id: "z" })).key).toBe("mjd_k");
  expect(post).toHaveBeenCalledWith(
    "/displays",
    { name: "TV", zone_id: "z" },
    { timeout: 8_000 },
  );
  await approveDisplay({ code: "123456" });
  expect(post).toHaveBeenLastCalledWith(
    "/displays/approve",
    { code: "123456" },
    { timeout: 8_000 },
  );
  const patch = spyOn(api, "patch").mockResolvedValue(
    ok({ display: { id: "d 1" } }),
  );
  await updateDisplay("d 1", { zone_id: null });
  expect(patch).toHaveBeenCalledWith(
    "/displays/d%201",
    { zone_id: null },
    { timeout: 8_000 },
  );
  const remove = spyOn(api, "delete").mockResolvedValue(ok(null, 204));
  await removeDisplay("d1");
  expect(remove).toHaveBeenCalledWith("/displays/d1", { timeout: 8_000 });
});

test("a removed display is told to pair again", () => {
  expect(isDisplayUnpaired({ response: { status: 403 } })).toBeTrue();
  expect(isDisplayUnpaired({ response: { status: 401 } })).toBeTrue();
  expect(isDisplayUnpaired({ response: { status: 500 } })).toBeFalse();
  expect(isDisplayUnpaired(new Error("offline"))).toBeFalse();
});

test("links keep the key in the fragment and screens read it back", () => {
  const link = displayLink("https://music.home:1993/", "mjd_a-b_c");
  expect(link).toBe("https://music.home:1993/display#key=mjd_a-b_c");
  expect(displayKeyFromHash(new URL(link).hash)).toBe("mjd_a-b_c");
  expect(displayKeyFromHash("#key=not-a-display-key")).toBeNull();
  expect(displayKeyFromHash("")).toBeNull();
});

test("asset paths resolve against the server, not the API prefix", () => {
  expect(displayServerOrigin("https://m.test/api/v1")).toBe("https://m.test");
  expect(displayServerOrigin("https://m.test/api/music/v1/")).toBe(
    "https://m.test",
  );
  expect(displayAssetURL("/media/images/a?x=1", "https://m.test")).toBe(
    "https://m.test/media/images/a?x=1",
  );
  expect(displayAssetURL("https://cdn.test/logo.png", "https://m.test")).toBe(
    "https://cdn.test/logo.png",
  );
  expect(displayAssetURL(null, "https://m.test")).toBeNull();
});

test("positions move with the clock only while playing, and stop at the end", () => {
  const playing = state();
  expect(estimateDisplayPosition(playing, 1_000, 3_500)).toBe(62_500);
  expect(estimateDisplayPosition(playing, 1_000, 10_000_000)).toBe(180_000);
  const paused = state({ zone: zone({ status: "paused" }) });
  expect(estimateDisplayPosition(paused, 1_000, 9_000)).toBe(60_000);
  expect(estimateDisplayPosition(state({ track: null }), 0, 5)).toBe(0);
});

test("screens ask often near a track's end and rarely when idle", () => {
  expect(displayPollDelay(null, 0)).toBe(4_000);
  expect(displayPollDelay(state({ status: "nothing" }), 0)).toBe(4_000);
  expect(displayPollDelay(state(), 60_000)).toBe(2_000);
  expect(displayPollDelay(state(), 178_000)).toBe(700);
  expect(displayPollDelay(state({ zone: zone({ status: "paused" }) }), 0)).toBe(
    3_000,
  );
});

test("the quality chip reads the file and, where planned, the path", () => {
  expect(displayQualityChip(track(), zone())).toEqual({
    format: "FLAC 96 kHz/24-bit",
    tier: "Hi-Res",
    path: null,
    light: "lossless",
  });
  expect(
    displayQualityChip(
      track(),
      zone({ quality: "lossless", bit_perfect: true }),
    ),
  ).toMatchObject({ path: "Bit-perfect", light: "lossless" });
  expect(
    displayQualityChip(track(), zone({ quality: "high", bit_perfect: false })),
  ).toMatchObject({ path: "High quality", light: "high" });
  expect(
    displayQualityChip(
      track({
        format: {
          label: "MP3 320 kbps",
          tier: "lossy",
          codec: "mp3",
          sample_rate: 44_100,
          bit_depth: null,
          channels: 2,
        },
      }),
      zone(),
    ),
  ).toMatchObject({ tier: "Lossy", light: "low" });
  expect(displayQualityChip(track({ format: null }), zone())).toBeNull();
  expect(displayQualityChip(null, zone())).toBeNull();
});

test("waveform bars keep each span's loudest peak", () => {
  expect(waveformBars({ peaks: [0, 255, 51, 102] }, 2)).toEqual([1, 0.4]);
  expect(waveformBars({ peaks: [255] }, 3)).toEqual([1, 1, 1]);
  expect(waveformBars(null, 10)).toEqual([]);
  // A quiet recording is drawn against its own loudest moment.
  expect(waveformBars({ peaks: [10, 50, 100] }, 3)).toEqual([0.1, 0.5, 1]);
  // Near-silence is not blown up to full height.
  expect(waveformBars({ peaks: [8, 16] }, 2)).toEqual([0.25, 0.5]);
});

test("the layout drifts a few pixels over minutes, never under reduced motion", () => {
  const first = displayDrift(0);
  const later = displayDrift(4 * 60_000 + 1);
  expect(first).not.toEqual(later);
  for (const at of [0, 1e6, 5e7, 9e9]) {
    const { x, y } = displayDrift(at, { maxPx: 10 });
    expect(Math.abs(x)).toBeLessThanOrEqual(10);
    expect(Math.abs(y)).toBeLessThanOrEqual(6);
  }
  expect(displayDrift(9e9, { reducedMotion: true })).toEqual({ x: 0, y: 0 });
  expect(displayDrift(60_000)).toEqual(displayDrift(120_000));
});

test("captions and track lines say what the screen shows", () => {
  expect(displayCaption(null)).toBe("Connecting…");
  expect(displayCaption(state())).toBe("Living room");
  expect(displayCaption(state({ status: "zone_missing" }))).toContain(
    "switched off",
  );
  expect(displayCaption(state({ status: "nothing" }))).toContain("any zone");
  expect(displayTrackLine(track())).toBe("Track 2 of 32 · 2012");
  expect(displayTrackLine(track({ track_count: null, year: null }))).toBe(
    "Track 2",
  );
  // Track 11 of a four-track partial copy reads "Track 11".
  expect(displayTrackLine(track({ track_number: 11, track_count: 4 }))).toBe(
    "Track 11 · 2012",
  );
  expect(formatDisplayTime(0)).toBe("0:00");
  expect(formatDisplayTime(187_400)).toBe("3:07");
  expect(formatDisplayTime(3_765_000)).toBe("1:02:45");
});
