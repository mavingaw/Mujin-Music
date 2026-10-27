import { expect, test } from "bun:test";
import { artistKicker, mainArtistDetail } from "./artist-roles";

test("an artist who also composes every song is headed Artist, not Composer", () => {
  expect(
    artistKicker({ ownsAlbums: true, isComposer: true, inLibrary: true }),
  ).toBe("Artist");
  expect(
    artistKicker({
      ownsAlbums: true,
      isComposer: true,
      inLibrary: true,
      kind: "Group",
    }),
  ).toBe("Group");
});

test("people known only from credits keep their role", () => {
  expect(
    artistKicker({ ownsAlbums: false, isComposer: true, inLibrary: false }),
  ).toBe("Composer");
  expect(
    artistKicker({ ownsAlbums: false, isComposer: false, inLibrary: false }),
  ).toBe("Contributor");
  expect(
    artistKicker({
      ownsAlbums: false,
      isComposer: false,
      inLibrary: true,
      kind: "Orchestra",
    }),
  ).toBe("Orchestra");
});

test("the Main artist chip counts albums and tracks", () => {
  expect(mainArtistDetail({ albums: 2, tracks: 8 })).toBe(
    "2 albums · 8 tracks",
  );
  expect(mainArtistDetail({ albums: 1, tracks: 1 })).toBe("1 album · 1 track");
  expect(mainArtistDetail({ albums: 1, tracks: 0 })).toBe("1 album");
});
