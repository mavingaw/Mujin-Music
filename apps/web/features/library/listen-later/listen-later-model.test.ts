import { expect, test } from "bun:test";
import type { ListenLaterItem } from "@parson/music-sdk";
import { listenLaterSummary } from "./listen-later-model";

const album = (songCount: number): ListenLaterItem => ({
  entityType: "album",
  entityId: `al-${songCount}`,
  addedAt: "2026-10-01T00:00:00Z",
  album: {
    id: `al-${songCount}`,
    name: "Album",
    artistId: "ar",
    artistName: "Artist",
    coverPath: "",
    releaseYear: "2020",
    songCount,
    quality: null,
  },
});

test("the summary counts kinds and songs", () => {
  expect(
    listenLaterSummary({ all: 0, album: 0, track: 0, artist: 0 }, []),
  ).toBe("Music you set aside, in one place.");
  expect(
    listenLaterSummary({ all: 3, album: 2, track: 0, artist: 1 }, [
      album(10),
      album(1),
    ]),
  ).toBe("2 albums · 1 artist · 11 songs to hear");
  expect(
    listenLaterSummary({ all: 1, album: 1, track: 0, artist: 0 }, [album(1)]),
  ).toBe("1 album · 1 song to hear");
});
