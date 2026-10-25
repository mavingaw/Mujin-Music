import { isSongExcludedByBans, type RadioBanIndex } from "@parson/music-sdk";
import type { QueueItem } from "./player-model";

/**
 * Appends Radio picks, skipping songs already queued from `fromIndex` on.
 * Pass the index after the current item to let a song that already played
 * come back (endless Radio recycles the library once it is used up).
 */
export function appendRadioPicks(
  items: readonly QueueItem[],
  picks: readonly QueueItem[],
  fromIndex = 0,
): QueueItem[] {
  const seen = new Set(
    items.slice(Math.max(0, fromIndex)).map((item) => item.song.id),
  );
  const additions = picks.filter((pick) => {
    if (!pick.song.id || seen.has(pick.song.id)) return false;
    seen.add(pick.song.id);
    return true;
  });
  return additions.length ? [...items, ...additions] : [...items];
}

/**
 * Removes an upcoming item and returns the queue with the active index
 * adjusted. The active item itself is never removed.
 */
export function removeUpcomingItem(
  items: readonly QueueItem[],
  activeIndex: number,
  removeIndex: number,
): { items: QueueItem[]; activeIndex: number } | null {
  if (
    removeIndex === activeIndex ||
    removeIndex < 0 ||
    removeIndex >= items.length
  ) {
    return null;
  }
  return {
    items: items.filter((_, index) => index !== removeIndex),
    activeIndex: removeIndex < activeIndex ? activeIndex - 1 : activeIndex,
  };
}

/** Drops the trailing Radio section (used when Radio is switched off). */
export function withoutRadioSection(
  items: readonly QueueItem[],
  sectionStart: number,
): QueueItem[] {
  return sectionStart < 0 ? [...items] : items.slice(0, sectionStart);
}

export function markRadioPickLiked(
  items: readonly QueueItem[],
  index: number,
): QueueItem[] {
  return items.map((item, itemIndex) =>
    itemIndex === index ? { ...item, liked: true } : item,
  );
}

/** Songs Radio should not pick again: the queue plus anything just rejected. */
export function radioExclusions(
  items: readonly QueueItem[],
  rejected: Iterable<string> = [],
): string[] {
  return [
    ...new Set([...items.map((item) => item.song.id), ...rejected]),
  ].filter(Boolean);
}

/** Whether bans keep this queue item out of Radio and Shuffle. */
export function queueItemBanned(
  index: RadioBanIndex,
  item: QueueItem,
): boolean {
  return isSongExcludedByBans(index, {
    songId: item.song.id,
    albumId: item.album?.id || item.song.album_object?.id,
    artistIds: [
      item.artist?.id,
      item.song.artist_object?.id,
      ...(item.song.contributing_artist_ids ?? []),
    ],
  });
}

/**
 * Applies a ban change to the queue at once: upcoming Radio picks that are
 * now banned are dropped (explicitly queued songs stay, as in Roon).
 * `currentBanned` reports whether the playing item is a banned Radio pick,
 * so the caller can skip it.
 */
export function withoutBannedRadioPicks(
  items: readonly QueueItem[],
  activeIndex: number,
  index: RadioBanIndex,
): { items: QueueItem[]; removed: number; currentBanned: boolean } {
  const kept = items.filter(
    (item, itemIndex) =>
      itemIndex <= activeIndex ||
      item.origin !== "generated" ||
      !queueItemBanned(index, item),
  );
  const current = items[activeIndex];
  return {
    items: kept,
    removed: items.length - kept.length,
    currentBanned: Boolean(
      current &&
      current.origin === "generated" &&
      queueItemBanned(index, current),
    ),
  };
}
