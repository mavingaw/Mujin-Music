import api from "../core/http";
import type { FocusAlbum, FocusSong } from "./focus";
import type { LibraryCatalogArtist } from "./library";

/** What Listen Later can hold. */
export type ListenLaterEntity = "album" | "track" | "artist";

export type ListenLaterSort =
  "added" | "title" | "artist" | "year" | "duration" | "quality";

export interface ListenLaterQuery {
  /** One kind, or every kind when omitted. */
  type?: ListenLaterEntity | "all";
  sort?: ListenLaterSort;
  direction?: "asc" | "desc";
  /** Text matched against titles, artists and albums. */
  q?: string;
  offset?: number;
  limit?: number;
}

export interface ListenLaterItem {
  entityType: ListenLaterEntity;
  entityId: string;
  /** ISO 8601 instant, UTC. */
  addedAt: string;
  album?: FocusAlbum;
  song?: FocusSong;
  artist?: LibraryCatalogArtist;
}

export interface ListenLaterCounts {
  all: number;
  album: number;
  track: number;
  artist: number;
}

export interface ListenLaterPage {
  /** Items matching the type and text filter. */
  total: number;
  offset: number;
  /** Items per kind before the type filter. */
  counts: ListenLaterCounts;
  /** Saved items the library no longer has. */
  unavailable: number;
  removeWhenPlayed: boolean;
  items: ListenLaterItem[];
}

const MAX_BATCH = 500;
const EMPTY_COUNTS: ListenLaterCounts = {
  all: 0,
  album: 0,
  track: 0,
  artist: 0,
};

export const LISTEN_LATER_SORTS: { value: ListenLaterSort; label: string }[] = [
  { value: "added", label: "Date added" },
  { value: "title", label: "Title" },
  { value: "artist", label: "Artist" },
  { value: "year", label: "Release year" },
  { value: "duration", label: "Length" },
  { value: "quality", label: "Quality" },
];

export async function getListenLater(
  query: ListenLaterQuery = {},
): Promise<ListenLaterPage> {
  const params: Record<string, string | number> = {};
  if (query.type && query.type !== "all") params.type = query.type;
  if (query.sort) params.sort = query.sort;
  if (query.direction) params.direction = query.direction;
  if (query.q?.trim()) params.q = query.q.trim();
  if (query.offset) params.offset = query.offset;
  if (query.limit) params.limit = query.limit;
  const response = await api.get<ListenLaterPage>("/library/listen-later", {
    params,
  });
  const page = response.data;
  return {
    total: page?.total ?? 0,
    offset: page?.offset ?? 0,
    counts: page?.counts ?? EMPTY_COUNTS,
    unavailable: page?.unavailable ?? 0,
    removeWhenPlayed: Boolean(page?.removeWhenPlayed),
    items: Array.isArray(page?.items) ? page.items : [],
  };
}

async function changeItems(
  path: string,
  entityType: ListenLaterEntity,
  entityIds: string[],
  field: "added" | "removed",
): Promise<number> {
  const unique = Array.from(new Set(entityIds.filter(Boolean)));
  let changed = 0;
  for (let index = 0; index < unique.length; index += MAX_BATCH) {
    const response = await api.post<Record<string, number>>(path, {
      entity_type: entityType,
      entity_ids: unique.slice(index, index + MAX_BATCH),
    });
    changed += response.data?.[field] ?? 0;
  }
  return changed;
}

/** Saves albums, tracks or artists for later; returns how many were new. */
export function addToListenLater(
  entityType: ListenLaterEntity,
  entityIds: string[],
): Promise<number> {
  return changeItems(
    "/library/listen-later/items",
    entityType,
    entityIds,
    "added",
  );
}

export function removeFromListenLater(
  entityType: ListenLaterEntity,
  entityIds: string[],
): Promise<number> {
  return changeItems(
    "/library/listen-later/items/remove",
    entityType,
    entityIds,
    "removed",
  );
}

/** Which of the given ids are saved for later. */
export async function getListenLaterSaved(
  entityType: ListenLaterEntity,
  entityIds: string[],
): Promise<Set<string>> {
  const unique = Array.from(new Set(entityIds.filter(Boolean)));
  const saved = new Set<string>();
  for (let index = 0; index < unique.length; index += MAX_BATCH) {
    const response = await api.post<{ ids: string[] }>(
      "/library/listen-later/saved",
      {
        entity_type: entityType,
        entity_ids: unique.slice(index, index + MAX_BATCH),
      },
      { timeout: 4_000 },
    );
    for (const id of response.data?.ids ?? []) saved.add(id);
  }
  return saved;
}

export async function isInListenLater(
  entityType: ListenLaterEntity,
  entityId: string,
): Promise<boolean> {
  return (await getListenLaterSaved(entityType, [entityId])).has(entityId);
}

export async function setListenLaterRemoveWhenPlayed(
  enabled: boolean,
): Promise<boolean> {
  const response = await api.put<{ remove_when_played: boolean }>(
    "/library/listen-later/settings",
    { remove_when_played: enabled },
  );
  return Boolean(response.data?.remove_when_played);
}

/** Display title and subtitle of a saved item. */
export function listenLaterLabels(item: ListenLaterItem): {
  title: string;
  subtitle: string;
} {
  if (item.album) {
    return {
      title: item.album.name,
      subtitle: [item.album.artistName, item.album.releaseYear]
        .filter(Boolean)
        .join(" · "),
    };
  }
  if (item.song) {
    return {
      title: item.song.name,
      subtitle: [item.song.artistName, item.song.albumName]
        .filter(Boolean)
        .join(" · "),
    };
  }
  if (item.artist) {
    const albums = item.artist.albumCount;
    return {
      title: item.artist.name,
      subtitle: `Artist · ${albums} ${albums === 1 ? "album" : "albums"}`,
    };
  }
  return { title: "Unavailable", subtitle: "" };
}

/** "Added today", "Added yesterday", "Added 3 days ago", "Added 12 Mar". */
export function addedAgo(addedAt: string, now: Date = new Date()): string {
  const added = new Date(addedAt);
  if (Number.isNaN(added.getTime())) return "";
  const startOfDay = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(added)) / 86_400_000);
  if (days <= 0) return "Added today";
  if (days === 1) return "Added yesterday";
  if (days < 7) return `Added ${days} days ago`;
  const sameYear = added.getFullYear() === now.getFullYear();
  return `Added ${added.toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  })}`;
}
