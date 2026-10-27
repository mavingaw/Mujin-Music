import {
  isSongExcludedByBans,
  type RadioBanIndex,
  radioMinimumUpcomingPicks,
  radioSectionStart,
  upcomingRadioPicks,
} from "@parson/music-sdk";

type Song = { id: string };

/** Played tracks kept before the current one when Radio extends the queue. */
export const radioHistoryLimit = 100;

export function queueOrigins(
  queue: readonly Song[],
  radioIds: ReadonlySet<string>,
): ("manual" | "generated")[] {
  return queue.map((song) => (radioIds.has(song.id) ? "generated" : "manual"));
}

/** Where the Radio section starts in the queue (-1 when there is none). */
export function radioStartIndex(
  queue: readonly Song[],
  radioIds: ReadonlySet<string>,
  currentIndex: number,
): number {
  return radioSectionStart(queueOrigins(queue, radioIds), currentIndex);
}

/** True when the current track is part of the Radio section. */
export function isCurrentRadioPick(
  queue: readonly Song[],
  radioIds: ReadonlySet<string>,
  currentIndex: number,
): boolean {
  return (
    currentIndex >= 0 &&
    radioStartIndex(queue, radioIds, currentIndex - 1) === currentIndex
  );
}

/** Whether to fetch more Radio picks so the Radio section stays visible. */
export function needsRadioTopUp(
  queue: readonly Song[],
  radioIds: ReadonlySet<string>,
  currentIndex: number,
): boolean {
  if (currentIndex < 0 || currentIndex >= queue.length) return false;
  const origins = queueOrigins(queue, radioIds);
  const radioUpcoming = upcomingRadioPicks(origins, currentIndex);
  const manualUpcoming = queue.length - currentIndex - 1 - radioUpcoming;
  return radioUpcoming < radioMinimumUpcomingPicks && manualUpcoming <= 2;
}

/** Appends songs that are not already queued. */
export function appendUnique<T extends Song>(
  queue: readonly T[],
  songs: readonly T[],
): T[] {
  const seen = new Set(queue.map((song) => song.id));
  return [
    ...queue,
    ...songs.filter((song) => {
      if (!song.id || seen.has(song.id)) return false;
      seen.add(song.id);
      return true;
    }),
  ];
}

/**
 * Appends Radio picks after the current queue, dropping the oldest played
 * tracks so an endless station does not grow without bound. Songs that
 * already played may come back (Radio recycles the library once it is used
 * up); only the current track and anything after it are never repeated.
 */
export function extendWithRadio<T extends Song>(
  queue: readonly T[],
  currentIndex: number,
  picks: readonly T[],
  historyLimit = radioHistoryLimit,
): { queue: T[]; nextIndex: number } {
  const trim = Math.max(0, Math.min(currentIndex, queue.length) - historyLimit);
  const kept = queue.slice(trim);
  const from = Math.max(0, currentIndex - trim);
  const additions = appendUnique(kept.slice(from), picks).slice(
    kept.length - from,
  );
  return {
    queue: [...kept, ...additions],
    nextIndex: kept.length,
  };
}

type BannableQueueSong = Song & {
  album_object?: { id?: string } | null;
  artist_object?: { id?: string } | null;
  contributing_artist_ids?: readonly string[] | null;
};

/** Whether bans keep this song out of Radio and Shuffle. */
export function songBanned(
  index: RadioBanIndex,
  song: BannableQueueSong,
): boolean {
  return isSongExcludedByBans(index, {
    songId: song.id,
    albumId: song.album_object?.id,
    artistIds: [
      song.artist_object?.id,
      ...(song.contributing_artist_ids ?? []),
    ],
  });
}

/**
 * Applies a ban change to the queue at once: upcoming Radio picks that are
 * now banned are dropped; songs you queued yourself stay. `currentBanned`
 * says whether the playing song is a banned Radio pick to skip.
 */
export function withoutBannedRadioPicks<T extends BannableQueueSong>(
  queue: readonly T[],
  radioIds: ReadonlySet<string>,
  currentIndex: number,
  index: RadioBanIndex,
): { queue: T[]; removedIds: string[]; currentBanned: boolean } {
  const removedIds: string[] = [];
  const kept = queue.filter((song, songIndex) => {
    if (songIndex <= currentIndex || !radioIds.has(song.id)) return true;
    if (!songBanned(index, song)) return true;
    removedIds.push(song.id);
    return false;
  });
  const current = queue[currentIndex];
  return {
    queue: kept,
    removedIds,
    currentBanned: Boolean(
      current && radioIds.has(current.id) && songBanned(index, current),
    ),
  };
}
