/**
 * Pure queue edits for the local player. Like Roon, and like the server's
 * zone queue, only upcoming items (after the current track) are removed,
 * cleared or shuffled; history and the playing track stay where they are.
 */

/** Removes upcoming item `index`; returns `null` when it is not upcoming. */
export function removeUpcomingItem<T>(
  items: readonly T[],
  current: number,
  index: number,
): T[] | null {
  if (!Number.isInteger(index) || index <= current || index >= items.length) {
    return null;
  }
  return items.filter((_, position) => position !== index);
}

/**
 * "Play next": `additions` go right after the current track and every item
 * already queued keeps its place after them.
 */
export function insertAfterCurrent<T>(
  items: readonly T[],
  current: number,
  additions: readonly T[],
): T[] {
  const at = Math.min(Math.max(0, current + 1), items.length);
  return [...items.slice(0, at), ...additions, ...items.slice(at)];
}

/** Keeps history and the current track; drops everything after it. */
export function clearUpcomingItems<T>(items: readonly T[], current: number) {
  return items.slice(0, Math.max(0, current + 1));
}

/** Shuffles only the items after `current` (Fisher-Yates). */
export function shuffleUpcomingItems<T>(
  items: readonly T[],
  current: number,
  random: () => number = Math.random,
): T[] {
  const start = Math.max(0, current + 1);
  const upcoming = items.slice(start);
  for (let index = upcoming.length - 1; index > 0; index -= 1) {
    const target = Math.floor(random() * (index + 1));
    [upcoming[index], upcoming[target]] = [upcoming[target]!, upcoming[index]!];
  }
  return [...items.slice(0, start), ...upcoming];
}

/**
 * Puts the upcoming items back in the order they had in `original`
 * (compared by `key`, so a queue the server re-created still matches).
 * Items added while shuffled keep their place at the end.
 */
export function unshuffleUpcomingItems<T, K = T>(
  items: readonly T[],
  current: number,
  original: readonly T[],
  key: (item: T) => K = (item) => item as unknown as K,
): T[] {
  const start = Math.max(0, current + 1);
  const rank = new Map<K, number>();
  original.forEach((item, index) => {
    const id = key(item);
    if (!rank.has(id)) rank.set(id, index);
  });
  const upcoming = items
    .slice(start)
    .map((item, index) => ({
      item,
      index,
      rank: rank.get(key(item)) ?? Number.MAX_SAFE_INTEGER,
    }))
    .sort((left, right) => left.rank - right.rank || left.index - right.index)
    .map(({ item }) => item);
  return [...items.slice(0, start), ...upcoming];
}
