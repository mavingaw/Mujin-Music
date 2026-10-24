import {
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
 * tracks so an endless station does not grow without bound.
 */
export function extendWithRadio<T extends Song>(
  queue: readonly T[],
  currentIndex: number,
  picks: readonly T[],
  historyLimit = radioHistoryLimit,
): { queue: T[]; nextIndex: number } {
  const trim = Math.max(0, Math.min(currentIndex, queue.length) - historyLimit);
  const kept = queue.slice(trim);
  return {
    queue: appendUnique(kept, picks),
    nextIndex: kept.length,
  };
}
