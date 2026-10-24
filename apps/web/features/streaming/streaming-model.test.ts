import { expect, test } from "bun:test";
import type { StreamingProviderStatus } from "@parson/music-sdk";
import {
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
