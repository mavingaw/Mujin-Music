import type {
  EnrichmentStatus,
  GenreRef,
  GenreTree,
  GenreTreeNode,
} from "../domain/types";

/**
 * A genre name as chips and tiles show it. MusicBrainz writes the genres it
 * lists in lower case ("industrial rock", "electro-industrial"); those read
 * as titles beside the canonical ones ("Industrial"). Names with capitals
 * of their own ("IDM", "Hip Hop", "J-Pop") are kept as they are.
 */
const GENRE_SMALL_WORDS = new Set(["and", "of", "the", "n", "in", "de", "et"]);

export function genreDisplayName(name: string): string {
  if (name !== name.toLowerCase()) return name;
  return name.replace(
    /(^|[\s\-/&(])(\p{L}+)/gu,
    (_, before: string, word: string, offset: number) =>
      before +
      (offset > 0 && GENRE_SMALL_WORDS.has(word)
        ? word
        : word.charAt(0).toUpperCase() + word.slice(1)),
  );
}

/** Route of a genre page in the web app. */
export function genreHref(slug: string) {
  return `/genre?slug=${encodeURIComponent(slug)}`;
}

const ORIGIN_LABELS: Record<string, string> = {
  tags: "file tags",
  musicbrainz: "MusicBrainz",
  user: "an edit",
};

/** "Jazz › Hard Bop", for a chip's tooltip and accessible name. */
export function genrePathLabel(genre: Pick<GenreRef, "name" | "path">) {
  return [...genre.path, genre.name].join(" › ");
}

/** "Jazz › Hard Bop · from MusicBrainz and file tags". */
export function genreChipTitle(genre: GenreRef) {
  const origins = genre.sources
    .map((source) => ORIGIN_LABELS[source] ?? source)
    .filter((label, index, all) => all.indexOf(label) === index);
  const from = origins.length
    ? ` · from ${origins.length > 1 ? `${origins.slice(0, -1).join(", ")} and ${origins[origins.length - 1]}` : origins[0]}`
    : "";
  return `${genrePathLabel(genre)}${from}`;
}

/** Where a set of chips came from, for a small caption next to them. */
export function genreOriginCaption(genres: readonly GenreRef[]) {
  const origins = new Set(genres.flatMap((genre) => genre.sources));
  if (origins.has("user")) return "Edited";
  if (origins.has("musicbrainz") && origins.has("tags"))
    return "MusicBrainz and file tags";
  if (origins.has("musicbrainz")) return "MusicBrainz";
  if (origins.has("tags")) return "File tags";
  return null;
}

export interface FlatGenre {
  slug: string;
  name: string;
  depth: number;
  album_count: number;
}

/** The tree in display order, each genre followed by its sub-genres. */
export function flattenGenreTree(nodes: readonly GenreTreeNode[], depth = 0) {
  const flat: FlatGenre[] = [];
  for (const node of nodes) {
    flat.push({
      slug: node.slug,
      name: node.name,
      depth,
      album_count: node.album_count,
    });
    flat.push(...flattenGenreTree(node.children, depth + 1));
  }
  return flat;
}

/**
 * Genre names to offer while editing an album's genres: canonical genres
 * and the library's own, matching `query` (case-insensitive, start of a
 * word first), without the ones already chosen.
 */
export function genreSuggestions(
  tree: GenreTree | null | undefined,
  query: string,
  chosen: readonly string[],
  limit = 8,
) {
  const taken = new Set(chosen.map((name) => name.toLowerCase()));
  const names = new Map<string, string>();
  for (const genre of tree?.catalog ?? []) names.set(genre.slug, genre.name);
  for (const genre of flattenGenreTree(tree?.genres ?? []))
    names.set(genre.slug, genre.name);
  for (const genre of tree?.other ?? []) names.set(genre.slug, genre.name);
  const wanted = query.trim().toLowerCase();
  const scored: { name: string; score: number }[] = [];
  for (const name of names.values()) {
    const lower = name.toLowerCase();
    if (taken.has(lower)) continue;
    if (!wanted) {
      scored.push({ name, score: 2 });
      continue;
    }
    if (lower.startsWith(wanted)) scored.push({ name, score: 0 });
    else if (lower.split(/[\s-]+/).some((word) => word.startsWith(wanted)))
      scored.push({ name, score: 1 });
    else if (lower.includes(wanted)) scored.push({ name, score: 2 });
  }
  return scored
    .sort((left, right) =>
      left.score === right.score
        ? left.name.localeCompare(right.name)
        : left.score - right.score,
    )
    .slice(0, limit)
    .map((entry) => entry.name);
}

/** What a genre page's About text waits for, or null. */
export function genreAboutWaitingLabel(
  enrichment: EnrichmentStatus | null | undefined,
) {
  if (!enrichment?.enabled) return null;
  const task = enrichment.task;
  if (task?.status !== "pending") return null;
  return task.stage === "wikipedia"
    ? "Fetching the description from Wikipedia."
    : "Finding this genre on Wikidata.";
}
