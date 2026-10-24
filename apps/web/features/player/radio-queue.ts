import type { QueueItem } from "./player-model";

/** Appends Radio picks, skipping songs already in the queue. */
export function appendRadioPicks(
  items: readonly QueueItem[],
  picks: readonly QueueItem[],
): QueueItem[] {
  const seen = new Set(items.map((item) => item.song.id));
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
