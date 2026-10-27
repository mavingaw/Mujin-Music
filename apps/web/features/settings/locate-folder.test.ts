import { expect, test } from "bun:test";
import { locateStartFolders } from "./locate-folder";

test("Locate opens near the old folder, not at the top of the disk", () => {
  expect(
    locateStartFolders({
      path: "/mnt/music-disk/Old Library",
      available: false,
      locate_from: "/mnt/music-disk",
    }),
  ).toEqual(["/mnt/music-disk"]);
  // An older server doesn't say: try the old path's parents, nearest first.
  expect(
    locateStartFolders({ path: "/home/ana/Music/FLAC", available: false }),
  ).toEqual(["/home/ana/Music", "/home/ana", "/home", "/"]);
  expect(
    locateStartFolders({ path: "D:\\Music\\FLAC", available: false }),
  ).toEqual(["D:\\Music", "D:\\", "/"]);
});
