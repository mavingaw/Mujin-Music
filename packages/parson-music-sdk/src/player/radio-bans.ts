import type {
  RadioBan,
  RadioBanKind,
  RadioBanTarget,
} from "../resources/radio-bans";

/** Fast lookups over a listener's bans. */
export interface RadioBanIndex {
  songs: ReadonlySet<string>;
  albums: ReadonlySet<string>;
  artists: ReadonlySet<string>;
}

export const emptyRadioBanIndex: RadioBanIndex = {
  songs: new Set(),
  albums: new Set(),
  artists: new Set(),
};

export function radioBanIndex(
  bans: readonly RadioBanTarget[] | null | undefined,
): RadioBanIndex {
  if (!bans?.length) return emptyRadioBanIndex;
  const songs = new Set<string>();
  const albums = new Set<string>();
  const artists = new Set<string>();
  for (const ban of bans) {
    if (!ban.id) continue;
    if (ban.kind === "song") songs.add(ban.id);
    else if (ban.kind === "album") albums.add(ban.id);
    else if (ban.kind === "artist") artists.add(ban.id);
  }
  return { songs, albums, artists };
}

/** Whether this exact item is banned (not whether its parent is). */
export function isRadioBanned(
  index: RadioBanIndex,
  target: RadioBanTarget,
): boolean {
  const set =
    target.kind === "song"
      ? index.songs
      : target.kind === "album"
        ? index.albums
        : index.artists;
  return Boolean(target.id) && set.has(target.id);
}

export interface BannableSong {
  songId?: string | null;
  albumId?: string | null;
  artistIds?: readonly (string | null | undefined)[];
}

/**
 * Whether Radio and Shuffle must skip a song: the song, its album or any of
 * its artists is banned. Mirrors the server's rule.
 */
export function isSongExcludedByBans(
  index: RadioBanIndex,
  song: BannableSong,
): boolean {
  if (song.songId && index.songs.has(song.songId)) return true;
  if (song.albumId && index.albums.has(song.albumId)) return true;
  return (song.artistIds ?? []).some(
    (artistId) => Boolean(artistId) && index.artists.has(artistId as string),
  );
}

/** Keeps only the songs Radio and Shuffle may play. */
export function withoutBannedSongs<T>(
  songs: readonly T[],
  index: RadioBanIndex,
  describe: (song: T) => BannableSong,
): T[] {
  if (!index.songs.size && !index.albums.size && !index.artists.size) {
    return [...songs];
  }
  return songs.filter((song) => !isSongExcludedByBans(index, describe(song)));
}

/** The heart's three states, as in Roon: not favourite, favourite, banned. */
export type HeartState = "none" | "liked" | "banned";

/** Next state when the heart is clicked: none → liked → banned → none. */
export function nextHeartState(state: HeartState): HeartState {
  return state === "none" ? "liked" : state === "liked" ? "banned" : "none";
}

export function heartState(liked: boolean, banned: boolean): HeartState {
  return banned ? "banned" : liked ? "liked" : "none";
}

const kindLabels: Record<RadioBanKind, string> = {
  song: "track",
  album: "album",
  artist: "artist",
};

export function radioBanKindLabel(kind: RadioBanKind): string {
  return kindLabels[kind];
}

/** Menu label: "Ban this artist" / "Unban this artist". */
export function radioBanActionLabel(
  kind: RadioBanKind,
  banned: boolean,
): string {
  return `${banned ? "Unban" : "Ban"} this ${kindLabels[kind]}`;
}

/** Applies a ban change to a cached list (for optimistic updates). */
export function applyRadioBanChange(
  bans: readonly RadioBan[],
  target: RadioBanTarget & { name?: string | null },
  banned: boolean,
): RadioBan[] {
  const rest = bans.filter(
    (ban) => !(ban.kind === target.kind && ban.id === target.id),
  );
  if (!banned) return rest;
  return [
    {
      kind: target.kind,
      id: target.id,
      name: target.name ?? null,
      created_at: new Date().toISOString(),
    },
    ...rest,
  ];
}

export type RadioBanFilter = "all" | RadioBanKind;

/** Bans grouped for the Library's "Banned" view. */
export function filterRadioBans(
  bans: readonly RadioBan[],
  filter: RadioBanFilter,
): RadioBan[] {
  return filter === "all"
    ? [...bans]
    : bans.filter((ban) => ban.kind === filter);
}
