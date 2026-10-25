import type { FacetValueOrder, FocusEntity } from "@parson/music-sdk";

/** Narrowest a facet column may get before a page holds one fewer. */
export const MIN_COLUMN_WIDTH = 168;
/** Space between columns, matching the panel's `gap-6`. */
export const COLUMN_GAP = 24;

/** A viewer's column layout for one browser (albums, songs, ...). */
export interface ColumnPrefs {
  /** Columns that stay at the front while the rest page past them. */
  pinned: string[];
  /** Columns the viewer removed from the panel. */
  hidden: string[];
  /** Value order per column; columns not listed read most frequent first. */
  order: Record<string, FacetValueOrder>;
}

export const EMPTY_PREFS: ColumnPrefs = { pinned: [], hidden: [], order: {} };

/** How many whole columns fit across `width` pixels; always at least one. */
export function columnsPerPage(
  width: number,
  minWidth = MIN_COLUMN_WIDTH,
  gap = COLUMN_GAP,
): number {
  if (!Number.isFinite(width) || width <= 0) return 1;
  return Math.max(1, Math.floor((width + gap) / (minWidth + gap)));
}

/** Pinned columns first (in pin order), then the rest; hidden ones apart. */
export function arrangeColumns<T>(
  columns: T[],
  keyOf: (column: T) => string,
  prefs: ColumnPrefs,
): { pinned: T[]; rest: T[]; hidden: T[] } {
  const hidden = new Set(prefs.hidden);
  const visible = columns.filter((column) => !hidden.has(keyOf(column)));
  const pinned = prefs.pinned
    .map((key) => visible.find((column) => keyOf(column) === key))
    .filter((column): column is T => column !== undefined);
  const pinnedKeys = new Set(pinned.map(keyOf));
  return {
    pinned,
    rest: visible.filter((column) => !pinnedKeys.has(keyOf(column))),
    hidden: columns.filter((column) => hidden.has(keyOf(column))),
  };
}

/**
 * The first column of each page: every `slots` columns, with the last page
 * ending flush with the last column so it is never short.
 */
export function pageStarts(count: number, slots: number): number[] {
  if (slots <= 0 || count <= slots) return [0];
  const starts: number[] = [];
  for (let start = 0; start < count - slots; start += slots) {
    starts.push(start);
  }
  starts.push(count - slots);
  return starts;
}

/**
 * Which columns one page shows. Pinned columns always show (leaving at
 * least one slot for paging when there is anything to page through); the
 * remaining slots show whole unpinned columns from `start`. Nothing is ever
 * partly visible: a page is a whole number of columns. The arrows step
 * between the page starts, so a start left between them (after columns
 * change) rejoins the grid on the next step.
 */
export function pageLayout(
  pinnedCount: number,
  restCount: number,
  perPage: number,
  start: number,
) {
  const pinnedShown = Math.min(
    pinnedCount,
    restCount > 0 ? Math.max(0, perPage - 1) : perPage,
  );
  const slots = restCount > 0 ? Math.max(1, perPage - pinnedShown) : 0;
  const maxStart = Math.max(0, restCount - slots);
  const first = Math.min(Math.max(0, start), maxStart);
  const starts = pageStarts(restCount, slots);
  let page = 0;
  starts.forEach((candidate, index) => {
    if (candidate <= first) page = index;
  });
  return {
    pinnedShown,
    slots,
    start: first,
    end: Math.min(restCount, first + slots),
    canPrevious: first > 0,
    canNext: first < maxStart,
    starts,
    /** Index into `starts` of the page being shown. */
    page,
    /** The start a page forward or back lands on. */
    next: starts.find((candidate) => candidate > first) ?? maxStart,
    previous: [...starts].reverse().find((candidate) => candidate < first) ?? 0,
  };
}

/**
 * Where paging resumes after the columns change (a click can add, remove or
 * reorder columns). The first page stays the first page, so Genres and
 * Release date never scroll away under a click; otherwise the column that
 * led the page leads it again, or the old position is kept if it is gone.
 */
export function resumeStart(
  previousStart: number,
  anchorKey: string | null,
  restKeys: string[],
): number {
  if (previousStart <= 0 || anchorKey === null) return 0;
  const index = restKeys.indexOf(anchorKey);
  return index === -1 ? previousStart : index;
}

export function togglePinned(prefs: ColumnPrefs, key: string): ColumnPrefs {
  return prefs.pinned.includes(key)
    ? { ...prefs, pinned: prefs.pinned.filter((item) => item !== key) }
    : {
        ...prefs,
        pinned: [...prefs.pinned, key],
        hidden: prefs.hidden.filter((item) => item !== key),
      };
}

export function toggleHidden(prefs: ColumnPrefs, key: string): ColumnPrefs {
  return prefs.hidden.includes(key)
    ? { ...prefs, hidden: prefs.hidden.filter((item) => item !== key) }
    : {
        ...prefs,
        hidden: [...prefs.hidden, key],
        pinned: prefs.pinned.filter((item) => item !== key),
      };
}

export function setValueOrder(
  prefs: ColumnPrefs,
  key: string,
  order: FacetValueOrder,
): ColumnPrefs {
  const next = { ...prefs.order };
  if (order === "count") delete next[key];
  else next[key] = order;
  return { ...prefs, order: next };
}

function storageKey(entity: FocusEntity) {
  return `parson.focus.columns.${entity}`;
}

function stringList(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

/** Parses stored prefs, dropping anything malformed. */
export function parseColumnPrefs(raw: string | null): ColumnPrefs {
  if (!raw) return EMPTY_PREFS;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return EMPTY_PREFS;
    const record = parsed as Record<string, unknown>;
    const order: Record<string, FacetValueOrder> = {};
    if (record.order && typeof record.order === "object") {
      for (const [key, value] of Object.entries(record.order)) {
        if (value === "name") order[key] = "name";
      }
    }
    return {
      pinned: stringList(record.pinned),
      hidden: stringList(record.hidden),
      order,
    };
  } catch {
    return EMPTY_PREFS;
  }
}

/** The viewer's layout, or the default when storage is unavailable. */
export function loadColumnPrefs(entity: FocusEntity): ColumnPrefs {
  try {
    return parseColumnPrefs(window.localStorage.getItem(storageKey(entity)));
  } catch {
    return EMPTY_PREFS;
  }
}

export function saveColumnPrefs(entity: FocusEntity, prefs: ColumnPrefs) {
  try {
    window.localStorage.setItem(storageKey(entity), JSON.stringify(prefs));
  } catch {
    // Layout is a convenience; the panel works without storage.
  }
}
