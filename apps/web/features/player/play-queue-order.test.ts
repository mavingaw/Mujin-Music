import { describe, expect, test } from "bun:test";
import type { Artist, LibraryAlbum } from "@parson/music-sdk";
import { albumQueue, artistQueue } from "./play-queue-order";

const song = (id: string) =>
  ({ id, name: id }) as LibraryAlbum["songs"][number];

describe("Play-now queues", () => {
  test("an album plays in album order with its artist", () => {
    const album = {
      id: "a",
      artist_object: { id: "x", name: "X" },
      songs: [song("1"), song("2")],
    } as unknown as LibraryAlbum;
    expect(albumQueue(album).map((item) => item.song.id)).toEqual(["1", "2"]);
    expect(albumQueue({ ...album, artist_object: undefined } as never)).toEqual(
      [],
    );
  });

  test("an artist plays oldest album first, undated last, each track once", () => {
    const artist = {
      id: "x",
      name: "X",
      albums: [
        { id: "undated", first_release_date: "", songs: [song("u")] },
        {
          id: "late",
          first_release_date: "2011",
          songs: [song("l"), song("shared")],
        },
        {
          id: "early",
          first_release_date: "1999-04",
          songs: [song("e"), song("shared")],
        },
      ],
    } as unknown as Artist;
    expect(artistQueue(artist).map((item) => item.song.id)).toEqual([
      "e",
      "shared",
      "l",
      "u",
    ]);
  });
});
