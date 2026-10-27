import type { LibraryRoot } from "@parson/music-sdk";

/**
 * Where "Locate…" starts browsing for a music folder that moved: the closest
 * folder above the old location that still exists on the server (the server
 * works it out), so the picker opens near the old path instead of at the top
 * of the disk. Older servers don't say, so the old path's parents are tried
 * from the nearest one up.
 */
export function locateStartFolders(root: LibraryRoot): string[] {
  if (root.locate_from) return [root.locate_from];
  return [...parentFolders(root.path), "/"].filter(
    (folder, index, all) => all.indexOf(folder) === index,
  );
}

function parentFolders(path: string): string[] {
  const windows = /^[a-z]:[\\/]/i.test(path) || path.includes("\\");
  const separator = windows ? "\\" : "/";
  const parts = path.split(/[\\/]+/).filter(Boolean);
  const folders: string[] = [];
  for (let length = parts.length - 1; length >= 1; length -= 1) {
    const joined = parts.slice(0, length).join(separator);
    folders.push(
      windows ? `${joined}${length === 1 ? separator : ""}` : `/${joined}`,
    );
  }
  return folders;
}
