import { expect, test } from "bun:test";
import type {
  StreamingDelivery,
  StreamingProviderStatus,
} from "@parson/music-sdk";
import {
  deliverySummary,
  needsOwnCredentials,
  serviceAction,
  formatTone,
  formatToneFromLabel,
  mergeLibraryAlbums,
  parseAlbumSourceFilter,
  providerStatusLine,
  streamingAlbumEntry,
  type LibraryAlbumEntry,
} from "./streaming-model";

const entry = (
  artistName: string,
  name: string,
  source: LibraryAlbumEntry["source"] = null,
): LibraryAlbumEntry => ({
  key: `${source ?? "local"}:${name}`,
  id: name,
  name,
  artistId: "",
  artistName,
  cover: "",
  year: "2024",
  source,
  formatLabel: null,
});

test("streaming albums interleave with local albums by artist", () => {
  const merged = mergeLibraryAlbums(
    [entry("Abba", "Arrival"), entry("Nina Simone", "Pastel Blues")],
    [entry("Mara Vell", "Harbor Lights", "tidal"), entry("Zed", "Z", "qobuz")],
    true,
  );
  expect(merged.map((album) => album.name)).toEqual([
    "Arrival",
    "Harbor Lights",
    "Pastel Blues",
    "Z",
  ]);
});

test("streaming albums past the loaded page wait for the next page", () => {
  const merged = mergeLibraryAlbums(
    [entry("Abba", "Arrival")],
    [
      entry("Mara Vell", "Harbor Lights", "tidal"),
      entry("Abba", "Ab", "qobuz"),
    ],
    false,
  );
  expect(merged.map((album) => album.name)).toEqual(["Ab", "Arrival"]);
  expect(
    mergeLibraryAlbums([], [entry("A", "B", "tidal")], false),
  ).toHaveLength(1);
});

test("library items map to badge-carrying album tiles", () => {
  const tile = streamingAlbumEntry({
    id: "qobuz:q1",
    provider: "qobuz",
    kind: "album",
    title: "Glass Orchard",
    artist_name: "Ilse Marten",
    album_title: null,
    cover_url: "https://static.qobuz.test/c.jpg",
    release_date: "2025-02-14",
    duration: 600,
    track_count: 3,
    format_label: "24/192 FLAC",
    origin: "parson",
    added_at: 1,
    item: null,
  });
  expect(tile).toMatchObject({
    id: "qobuz:q1",
    source: "qobuz",
    year: "2025",
    formatLabel: "24/192 FLAC",
  });
});

test("quality tones follow bit depth, sample rate and codec", () => {
  expect(
    formatTone({ lossless: true, bit_depth: 24, sample_rate: 96000 }),
  ).toBe("hi-res");
  expect(
    formatTone({ lossless: true, bit_depth: 16, sample_rate: 44100 }),
  ).toBe("lossless");
  expect(
    formatTone({ lossless: false, bit_depth: null, sample_rate: null }),
  ).toBe("lossy");
  expect(formatToneFromLabel("24/192 FLAC")).toBe("hi-res");
  expect(formatToneFromLabel("16/44.1 FLAC")).toBe("lossless");
  expect(formatToneFromLabel("AAC 320")).toBe("lossy");
  expect(parseAlbumSourceFilter("qobuz")).toBe("qobuz");
  expect(parseAlbumSourceFilter("spotify")).toBe("all");
});

test("service rows explain what to do next", () => {
  const base = {
    status: "configured",
    account_name: null,
    status_message: null,
    credentials: { configured: false },
  } as unknown as StreamingProviderStatus;
  expect(providerStatusLine(base)).toBe("Not configured");
  expect(
    providerStatusLine({
      ...base,
      credentials: { ...base.credentials, configured: true },
    }),
  ).toBe("Credentials saved. Not signed in.");
  expect(
    providerStatusLine({ ...base, status: "signed_in", account_name: "ada" }),
  ).toBe("Signed in as ada");
});

const status = (
  overrides: Partial<StreamingProviderStatus> = {},
): StreamingProviderStatus => ({
  id: "qobuz",
  name: "Qobuz",
  sign_in: "password",
  status: "configured",
  status_message: null,
  account_name: null,
  plan: null,
  quality: "hi_res_max",
  quality_options: [],
  last_library_sync: null,
  credentials: {
    configured: false,
    client_id_hint: null,
    has_secret: false,
    uses_server_credentials: false,
  },
  server_credentials_available: false,
  pending: null,
  library: { albums: 0, tracks: 0 },
  sandbox: false,
  ...overrides,
});

test("server credentials skip the credentials step", () => {
  const bare = status();
  expect(needsOwnCredentials(bare)).toBeTrue();
  expect(serviceAction(bare)).toBe("Set up");
  expect(providerStatusLine(bare)).toBe("Not configured");

  const withServer = status({ server_credentials_available: true });
  expect(needsOwnCredentials(withServer)).toBeFalse();
  expect(serviceAction(withServer)).toBe("Sign in");
  expect(providerStatusLine(withServer)).toBe("Not signed in");

  expect(serviceAction(status({ status: "signed_in" }))).toBe("Edit");
});

const flac = (bitDepth: number, sampleRate: number, label: string) => ({
  codec: "flac",
  lossless: true,
  bit_depth: bitDepth,
  sample_rate: sampleRate,
  bitrate_kbps: null,
  channels: 2,
  label,
});

const delivery = (
  overrides: Partial<StreamingDelivery> = {},
): StreamingDelivery => ({
  track_id: "tidal:300101",
  requested: "hi_res_max",
  announced: flac(24, 192000, "24/192 FLAC"),
  delivered: flac(24, 192000, "24/192 FLAC"),
  container: "mp4",
  frame_verified: true,
  matches_announced: true,
  sandbox: false,
  at: 0,
  ...overrides,
});

test("the player shows what the stream actually delivered", () => {
  const summary = deliverySummary(delivery());
  expect(summary.badge).toBe("24/192 FLAC");
  expect(summary.tone).toBe("hi-res");
  expect(summary.lines).toEqual([
    "Requested Max",
    "Delivered 24/192 FLAC, stereo, in MP4 segments",
    "First audio frame verified",
  ]);
  expect(summary.warning).toBeNull();

  const downgraded = deliverySummary(
    delivery({
      delivered: flac(16, 44100, "16/44.1 FLAC"),
      matches_announced: false,
      container: "flac",
    }),
  );
  expect(downgraded.badge).toBe("16/44.1 FLAC");
  expect(downgraded.tone).toBe("lossless");
  expect(downgraded.warning).toBe("The service announced 24/192 FLAC");

  const sandbox = deliverySummary(
    delivery({
      sandbox: true,
      matches_announced: false,
      delivered: { ...flac(16, 44100, "16/44.1 PCM"), codec: "pcm" },
      container: "wav",
      frame_verified: false,
    }),
  );
  expect(sandbox.badge).toBe("Preview 16/44.1 PCM");
  expect(sandbox.warning).toBeNull();
  expect(sandbox.lines.at(-1)).toBe("Sandbox preview tone, not a real stream");

  expect(deliverySummary(delivery({ delivered: null })).badge).toBe(
    "Format unknown",
  );
});
