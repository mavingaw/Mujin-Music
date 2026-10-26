"use client";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/cn";
import {
  cycleFilter,
  decadeCriterion,
  decadesFromYears,
  emptyFocus,
  facetBaseQuery,
  facetColumnKey,
  facetSelectionSummary,
  facetTitle,
  facetValueState,
  filterState,
  getFocusFacets,
  isYearFacet,
  orderedFacetValues,
  setFilter,
  toggleFacetValue,
  visibleFacets,
  type Facet,
  type FacetValueOrder,
  type FilterState,
  type FocusQuery,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Columns3,
  EyeOff,
  Minus,
  Pin,
  PinOff,
  Plus,
  Search,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import {
  COLUMN_GAP,
  EMPTY_PREFS,
  arrangeColumns,
  columnsPerPage,
  loadColumnPrefs,
  pageLayout,
  resumeStart,
  saveColumnPrefs,
  setValueOrder,
  toggleHidden,
  togglePinned,
  type ColumnPrefs,
} from "./focus-columns";
import { facetQueryKey } from "./focus-state";

/** Rows a column shows before "View more". */
const COLUMN_VALUES = 6;
/** Values fetched when a column is expanded; search reaches the rest. */
const EXPANDED_VALUES = 200;
const PHONE_QUERY = "(max-width: 639px)";
/** Facets a phone lists before "more". */
const PHONE_FACETS = 6;

interface PagePosition {
  start: number;
  /** The column that led the page, so paging survives column changes. */
  anchor: string | null;
}

/** Each browser's page position, so remounting the panel keeps it. */
const pageMemory = new Map<string, PagePosition>();

function subscribePhone(onChange: () => void) {
  const media = window.matchMedia(PHONE_QUERY);
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}

function usePhoneLayout() {
  return useSyncExternalStore(
    subscribePhone,
    () => window.matchMedia(PHONE_QUERY).matches,
    () => false,
  );
}

/** Width of an element, tracked as it resizes. */
function useWidth<T extends HTMLElement>() {
  const [node, setNode] = useState<T | null>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    if (!node) return;
    setWidth(node.clientWidth);
    const observer = new ResizeObserver(() => setWidth(node.clientWidth));
    observer.observe(node);
    return () => observer.disconnect();
  }, [node]);
  return [setNode, width] as const;
}

interface ColumnTools {
  order: FacetValueOrder;
  pinned: boolean;
  onOrder: (order: FacetValueOrder) => void;
  onPin: () => void;
  onHide: () => void;
}

export default function FocusPanel({
  query,
  setQuery,
}: {
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const base = facetBaseQuery(query);
  const facets = useQuery({
    queryKey: ["library", "focus", "facets", facetQueryKey(base)],
    queryFn: () => getFocusFacets(base, { limit: COLUMN_VALUES + 2 }),
    placeholderData: keepPreviousData,
    staleTime: 30_000,
  });
  const phone = usePhoneLayout();
  const [rangeFacet, setRangeFacet] = useState<Facet | null>(null);
  const [prefs, setPrefs] = useState<ColumnPrefs>(() =>
    loadColumnPrefs(query.entity),
  );
  useEffect(() => setPrefs(loadColumnPrefs(query.entity)), [query.entity]);
  const updatePrefs = useCallback(
    (change: (prefs: ColumnPrefs) => ColumnPrefs) =>
      setPrefs((current) => {
        const next = change(current);
        saveColumnPrefs(query.entity, next);
        return next;
      }),
    [query.entity],
  );

  const composerScoped = Boolean(
    query.scope?.some(
      (criterion) =>
        criterion.facet === "credit" && criterion.role === "composer",
    ),
  );
  const columns = useMemo(
    () =>
      visibleFacets(facets.data?.facets ?? [], query.entity, {
        composerScoped,
        query,
      }),
    [facets.data, query, composerScoped],
  );
  const arranged = useMemo(
    () => arrangeColumns(columns, facetColumnKey, prefs),
    [columns, prefs],
  );
  const toolsFor = (facet: Facet): ColumnTools => {
    const key = facetColumnKey(facet);
    return {
      order: prefs.order[key] ?? "count",
      pinned: prefs.pinned.includes(key),
      onOrder: (order) =>
        updatePrefs((current) => setValueOrder(current, key, order)),
      onPin: () => updatePrefs((current) => togglePinned(current, key)),
      onHide: () => updatePrefs((current) => toggleHidden(current, key)),
    };
  };

  const loading = facets.isPending && !facets.data;
  const empty = !loading && columns.length === 0;

  return (
    <section
      aria-label="Focus"
      className="relative rounded-2xl border border-white/10 bg-white/[0.025]"
      id="library-focus-panel"
    >
      <div className="flex items-center justify-between gap-2 border-b border-white/6 px-4 py-2 text-xs text-zinc-500 sm:px-5">
        <p className="min-w-0 truncate">
          <span className="sm:hidden">Tap to focus, again to exclude.</span>
          <span className="hidden sm:inline">
            Click to focus, click again to exclude.
          </span>
        </p>
        <div className="flex shrink-0 items-center gap-1.5">
          <ColumnsMenu
            columns={columns}
            prefs={prefs}
            onReset={() => updatePrefs(() => EMPTY_PREFS)}
            onToggleHidden={(key) =>
              updatePrefs((current) => toggleHidden(current, key))
            }
            onTogglePinned={(key) =>
              updatePrefs((current) => togglePinned(current, key))
            }
          />
          <div
            aria-label="Combine filters"
            className="flex rounded-full border border-white/10 p-0.5"
            role="radiogroup"
          >
            {(["all", "any"] as const).map((mode) => {
              const active = (query.match ?? "all") === mode;
              return (
                <button
                  aria-checked={active}
                  className={cn(
                    "whitespace-nowrap rounded-full px-2.5 py-0.5 text-xs transition-colors",
                    active
                      ? "bg-white text-black"
                      : "text-zinc-400 hover:text-white",
                  )}
                  key={mode}
                  onClick={() =>
                    setQuery({
                      ...query,
                      match: mode === "all" ? undefined : "any",
                    })
                  }
                  role="radio"
                  type="button"
                >
                  <span className="hidden sm:inline">Match </span>
                  {mode}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {loading ? (
        <div className="grid grid-cols-2 gap-6 px-5 py-5 sm:grid-cols-5">
          {Array.from({ length: 5 }, (_, index) => (
            <div
              className={cn("space-y-3", index > 1 && "hidden sm:block")}
              key={index}
            >
              <div className="h-3 w-20 rounded bg-white/8" />
              {Array.from({ length: 5 }, (__, row) => (
                <div className="h-3 w-28 rounded bg-white/5" key={row} />
              ))}
            </div>
          ))}
        </div>
      ) : empty ? (
        <p className="px-5 py-8 text-sm text-zinc-500">
          {facets.isError
            ? "Focus options could not be loaded."
            : "Nothing left to focus on. Remove a filter to see more options."}
        </p>
      ) : phone ? (
        <FacetAccordion
          columns={[...arranged.pinned, ...arranged.rest]}
          fetching={facets.isFetching}
          onCustomRange={setRangeFacet}
          query={query}
          setQuery={setQuery}
          toolsFor={toolsFor}
        />
      ) : (
        <FacetPages
          fetching={facets.isFetching}
          key={`${query.entity}:${JSON.stringify(query.scope ?? [])}`}
          memoryKey={`${query.entity}:${JSON.stringify(query.scope ?? [])}`}
          onCustomRange={setRangeFacet}
          pinned={arranged.pinned}
          query={query}
          rest={arranged.rest}
          setQuery={setQuery}
          toolsFor={toolsFor}
        />
      )}
      {!loading && !empty && arranged.hidden.length > 0 && (
        <p className="border-t border-white/6 px-4 py-2 text-xs text-zinc-500 sm:px-5">
          {arranged.hidden.length} hidden{" "}
          {arranged.hidden.length === 1 ? "column" : "columns"}:{" "}
          {arranged.hidden.map((facet, index) => (
            <span key={facetColumnKey(facet)}>
              {index > 0 && ", "}
              <button
                className="text-zinc-300 underline-offset-4 hover:text-white hover:underline"
                onClick={() =>
                  updatePrefs((current) =>
                    toggleHidden(current, facetColumnKey(facet)),
                  )
                }
                title={`Show ${facetTitle(facet)}`}
                type="button"
              >
                {facetTitle(facet)}
              </button>
            </span>
          ))}
        </p>
      )}
      <YearRangeDialog
        facet={rangeFacet}
        onOpenChange={(open) => !open && setRangeFacet(null)}
        query={query}
        setQuery={setQuery}
      />
    </section>
  );
}

/** Desktop: whole columns, paged with arrows in their own gutters. */
function FacetPages({
  fetching,
  memoryKey,
  onCustomRange,
  pinned,
  query,
  rest,
  setQuery,
  toolsFor,
}: {
  fetching: boolean;
  memoryKey: string;
  onCustomRange: (facet: Facet) => void;
  pinned: Facet[];
  query: FocusQuery;
  rest: Facet[];
  setQuery: (query: FocusQuery) => void;
  toolsFor: (facet: Facet) => ColumnTools;
}) {
  const [measure, width] = useWidth<HTMLDivElement>();
  const perPage = columnsPerPage(width);
  const [position, setPosition] = useState<PagePosition>(
    () => pageMemory.get(memoryKey) ?? { start: 0, anchor: null },
  );
  const restKeys = rest.map(facetColumnKey);
  const layout = pageLayout(
    pinned.length,
    rest.length,
    perPage,
    resumeStart(position.start, position.anchor, restKeys),
  );
  const goTo = (start: number) => {
    const next = { start, anchor: restKeys[start] ?? null };
    pageMemory.set(memoryKey, next);
    setPosition(next);
  };
  const shownPinned = pinned.slice(0, layout.pinnedShown);
  const shownRest = rest.slice(layout.start, layout.end);
  const total = pinned.length + rest.length;
  const paged = layout.canPrevious || layout.canNext;

  return (
    <div className="relative">
      <div className="grid grid-cols-[2.25rem_minmax(0,1fr)_2.25rem] items-stretch">
        <PageArrow
          direction="previous"
          disabled={!layout.canPrevious}
          onClick={() => goTo(layout.previous)}
        />
        <div
          className={cn(
            "grid min-w-0 py-5 transition-opacity",
            fetching && "opacity-80",
          )}
          ref={measure}
          style={{
            columnGap: COLUMN_GAP,
            gridTemplateColumns: `repeat(${perPage}, minmax(0, 1fr))`,
          }}
        >
          {[...shownPinned, ...shownRest].map((facet, index) => (
            <div
              className={cn(
                "group/column min-w-0",
                index === shownPinned.length - 1 &&
                  shownRest.length > 0 &&
                  "relative after:absolute after:-right-3 after:top-0 after:h-full after:w-px after:bg-white/8",
              )}
              data-focus-column={facetColumnKey(facet)}
              key={facetColumnKey(facet)}
            >
              <FacetColumn
                facet={facet}
                onCustomRange={() => onCustomRange(facet)}
                query={query}
                setQuery={setQuery}
                tools={toolsFor(facet)}
              />
            </div>
          ))}
        </div>
        <PageArrow
          direction="next"
          disabled={!layout.canNext}
          onClick={() => goTo(layout.next)}
        />
      </div>
      {paged && (
        <div className="-mt-2 flex items-center justify-center gap-1.5 pb-3">
          <span className="sr-only">
            Columns {layout.start + 1 + shownPinned.length}–
            {layout.end + shownPinned.length} of {total}
          </span>
          {layout.starts.map((start, index) => {
            const active = index === layout.page;
            return (
              <button
                aria-current={active ? "page" : undefined}
                aria-label={`Show ${rest[start] ? facetTitle(rest[start]) : "columns"} and after`}
                className={cn(
                  "h-1.5 rounded-full transition-all",
                  active
                    ? "w-5 bg-zinc-300"
                    : "w-1.5 bg-zinc-700 hover:bg-zinc-500",
                )}
                key={start}
                onClick={() => goTo(start)}
                type="button"
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

function PageArrow({
  direction,
  disabled,
  onClick,
}: {
  direction: "previous" | "next";
  disabled: boolean;
  onClick: () => void;
}) {
  const Icon = direction === "previous" ? ChevronLeft : ChevronRight;
  return (
    <div className="flex items-center justify-center">
      <button
        aria-label={
          direction === "previous"
            ? "Previous focus columns"
            : "More focus columns"
        }
        className={cn(
          "grid h-8 w-8 place-items-center rounded-full text-zinc-400 transition-colors hover:bg-white/8 hover:text-white",
          disabled && "pointer-events-none opacity-0",
        )}
        disabled={disabled}
        onClick={onClick}
        tabIndex={disabled ? -1 : undefined}
        type="button"
      >
        <Icon className="h-5 w-5" />
      </button>
    </div>
  );
}

/** Phone: one facet per row, opened in place. */
function FacetAccordion({
  columns,
  fetching,
  onCustomRange,
  query,
  setQuery,
  toolsFor,
}: {
  columns: Facet[];
  fetching: boolean;
  onCustomRange: (facet: Facet) => void;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
  toolsFor: (facet: Facet) => ColumnTools;
}) {
  const [open, setOpen] = useState<Set<string>>(
    () =>
      new Set(
        columns
          .filter((facet) => facetSelectionSummary(facet, query))
          .map(facetColumnKey),
      ),
  );
  const [all, setAll] = useState(false);
  // A phone shows the first facets and any with a filter; the rest wait
  // behind one row so the albums stay within reach.
  const shown = all
    ? columns
    : columns.filter(
        (facet, index) =>
          index < PHONE_FACETS || facetSelectionSummary(facet, query) !== "",
      );
  return (
    <ul
      className={cn(
        "divide-y divide-white/6 transition-opacity",
        fetching && "opacity-80",
      )}
    >
      {shown.map((facet) => {
        const key = facetColumnKey(facet);
        const expanded = open.has(key);
        const summary = facetSelectionSummary(facet, query);
        const tools = toolsFor(facet);
        return (
          <li key={key}>
            <button
              aria-controls={`focus-facet-${key}`}
              aria-expanded={expanded}
              className="flex w-full items-center gap-3 px-4 py-3 text-left"
              onClick={() =>
                setOpen((current) => {
                  const next = new Set(current);
                  if (next.has(key)) next.delete(key);
                  else next.add(key);
                  return next;
                })
              }
              type="button"
            >
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-zinc-400">
                  {tools.pinned && <Pin className="h-3 w-3" />}
                  {facetTitle(facet)}
                </span>
                <span
                  className={cn(
                    "truncate text-sm",
                    summary ? "text-white" : "text-zinc-500",
                  )}
                >
                  {summary || facetPreview(facet)}
                </span>
              </span>
              <ChevronDown
                className={cn(
                  "h-4 w-4 shrink-0 text-zinc-500 transition-transform",
                  expanded && "rotate-180",
                )}
              />
            </button>
            {expanded && (
              <div className="px-4 pb-4" id={`focus-facet-${key}`}>
                <div className="-mt-1 mb-1 flex items-center justify-end gap-1">
                  {!isYearFacet(facet.key) && (
                    <ColumnMenu facet={facet} tools={tools}>
                      <button
                        aria-label={`${facetTitle(facet)} options`}
                        className="flex h-7 items-center gap-1 rounded-full px-2 text-xs text-zinc-500 hover:bg-white/8 hover:text-white"
                        type="button"
                      >
                        {tools.order === "name" ? "A to Z" : "Most items"}
                        <ChevronDown className="h-3 w-3" />
                      </button>
                    </ColumnMenu>
                  )}
                  <ColumnToggles facet={facet} tools={tools} />
                </div>
                <FacetBody
                  facet={facet}
                  onCustomRange={() => onCustomRange(facet)}
                  order={tools.order}
                  query={query}
                  roomy
                  setQuery={setQuery}
                />
              </div>
            )}
          </li>
        );
      })}
      {columns.length > shown.length && (
        <li>
          <button
            className="flex w-full items-center justify-between px-4 py-3 text-left text-sm text-zinc-400 hover:text-white"
            onClick={() => setAll(true)}
            type="button"
          >
            <span>
              {columns.length - shown.length} more:{" "}
              <span className="text-zinc-500">
                {columns
                  .filter((facet) => !shown.includes(facet))
                  .slice(0, 3)
                  .map(facetTitle)
                  .join(", ")}
                …
              </span>
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-zinc-500" />
          </button>
        </li>
      )}
    </ul>
  );
}

/** "Rock, Pop, Jazz +27": a closed facet's most common values. */
function facetPreview(facet: Facet): string {
  if (isYearFacet(facet.key)) {
    const decades = decadesFromYears(facet.values);
    if (decades.length === 0) return "";
    const first = decades[0]!.decade;
    const last = decades[decades.length - 1]!.decade;
    return first === last ? `${first}s` : `${first}s – ${last}s`;
  }
  const { values, more } = orderedFacetValues(facet, emptyFocus(), {
    cap: 3,
  });
  const labels = values.map((value) => value.label).join(", ");
  return more > 0 ? `${labels} +${more.toLocaleString("en-US")}` : labels;
}

/** Show, hide and pin columns; lists every column the panel can show. */
function ColumnsMenu({
  columns,
  onReset,
  onToggleHidden,
  onTogglePinned,
  prefs,
}: {
  columns: Facet[];
  onReset: () => void;
  onToggleHidden: (key: string) => void;
  onTogglePinned: (key: string) => void;
  prefs: ColumnPrefs;
}) {
  if (columns.length === 0) return null;
  const customised =
    prefs.hidden.length > 0 ||
    prefs.pinned.length > 0 ||
    Object.keys(prefs.order).length > 0;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label="Choose focus columns"
          className="flex h-7 items-center gap-1.5 rounded-full px-2 text-xs text-zinc-400 hover:bg-white/8 hover:text-white"
          type="button"
        >
          <Columns3 className="h-3.5 w-3.5" />
          <span className="hidden sm:inline">Columns</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="max-h-[60vh] w-60 overflow-y-auto"
      >
        <DropdownMenuLabel>Show columns</DropdownMenuLabel>
        {columns.map((facet) => {
          const key = facetColumnKey(facet);
          const pinned = prefs.pinned.includes(key);
          return (
            <div className="flex items-center" key={key}>
              <DropdownMenuCheckboxItem
                checked={!prefs.hidden.includes(key)}
                className="flex-1"
                onSelect={(event) => {
                  event.preventDefault();
                  onToggleHidden(key);
                }}
              >
                {facetTitle(facet)}
              </DropdownMenuCheckboxItem>
              <button
                aria-label={`${pinned ? "Unpin" : "Pin"} ${facetTitle(facet)}`}
                aria-pressed={pinned}
                className={cn(
                  "grid h-8 w-8 shrink-0 place-items-center rounded-md hover:bg-white/8",
                  pinned ? "text-white" : "text-zinc-600 hover:text-zinc-300",
                )}
                onClick={() => onTogglePinned(key)}
                type="button"
              >
                <Pin className="h-3.5 w-3.5" />
              </button>
            </div>
          );
        })}
        {customised && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onReset}>
              Reset columns
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The column's menu: value order, pin and hide. */
function ColumnMenu({
  children,
  facet,
  tools,
}: {
  children: ReactNode;
  facet: Facet;
  tools: ColumnTools;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-52">
        <DropdownMenuLabel>{facetTitle(facet)}</DropdownMenuLabel>
        {!isYearFacet(facet.key) && (
          <>
            <DropdownMenuRadioGroup
              onValueChange={(value) => tools.onOrder(value as FacetValueOrder)}
              value={tools.order}
            >
              <DropdownMenuRadioItem value="count">
                Most items first
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="name">A to Z</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator />
          </>
        )}
        <DropdownMenuItem onSelect={tools.onPin}>
          {tools.pinned ? <PinOff /> : <Pin />}
          {tools.pinned ? "Unpin column" : "Pin to the front"}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={tools.onHide}>
          <EyeOff />
          Hide column
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Pin and hide toggles beside a column title. */
/**
 * Toggles that take no room until the column is hovered or focused, so a
 * long title such as "Conductor / Ensemble" is never cut short at rest.
 */
const REVEAL = "hidden group-hover/column:grid group-focus-within/column:grid";

function ColumnToggles({
  facet,
  tools,
  revealOnHover,
}: {
  facet: Facet;
  tools: ColumnTools;
  revealOnHover?: boolean;
}) {
  const title = facetTitle(facet);
  return (
    <span className="flex shrink-0 items-center">
      <button
        aria-label={`${tools.pinned ? "Unpin" : "Pin"} ${title}`}
        aria-pressed={tools.pinned}
        className={cn(
          "grid h-5 w-5 place-items-center rounded hover:bg-white/8",
          tools.pinned ? "text-zinc-200" : "text-zinc-600 hover:text-zinc-200",
          revealOnHover && !tools.pinned && REVEAL,
        )}
        onClick={tools.onPin}
        title={tools.pinned ? "Unpin column" : "Pin column to the front"}
        type="button"
      >
        <Pin className={cn("h-3 w-3", tools.pinned && "fill-current")} />
      </button>
      <button
        aria-label={`Hide ${title}`}
        className={cn(
          "grid h-5 w-5 place-items-center rounded text-zinc-600 hover:bg-white/8 hover:text-zinc-200",
          revealOnHover && REVEAL,
        )}
        onClick={tools.onHide}
        title="Hide column"
        type="button"
      >
        <EyeOff className="h-3 w-3" />
      </button>
    </span>
  );
}

function FacetValueButton({
  label,
  count,
  onClick,
  roomy,
  state,
}: {
  label: string;
  count: number;
  onClick: () => void;
  roomy?: boolean;
  state: FilterState;
}) {
  return (
    <button
      aria-pressed={state !== null}
      className={cn(
        "group flex w-full min-w-0 items-center gap-1.5 rounded-md text-left text-sm transition-colors",
        roomy ? "py-2" : "py-1",
        state === "include" && "font-medium text-white",
        state === "exclude" && "text-rose-200/90",
        state === null && "text-zinc-400 hover:text-white",
      )}
      onClick={onClick}
      title={
        state === "include"
          ? `${label}: focused. Click to exclude`
          : state === "exclude"
            ? `${label}: excluded. Click to clear`
            : `Focus on ${label}`
      }
      type="button"
    >
      <span className="grid h-4 w-4 shrink-0 place-items-center">
        {state === "include" && <Plus className="h-3.5 w-3.5" />}
        {state === "exclude" && <Minus className="h-3.5 w-3.5" />}
        {state === null && (
          <span className="h-1 w-1 rounded-full bg-zinc-700 group-hover:bg-zinc-400" />
        )}
      </span>
      <span
        className={cn(
          "truncate",
          state === "exclude" && "line-through decoration-rose-300/60",
        )}
      >
        {label}
      </span>
      <span className="shrink-0 text-xs tabular-nums text-zinc-600">
        {count.toLocaleString("en-US")}
      </span>
    </button>
  );
}

function FacetColumn({
  facet,
  onCustomRange,
  query,
  setQuery,
  tools,
}: {
  facet: Facet;
  onCustomRange: () => void;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
  tools: ColumnTools;
}) {
  return (
    <div className="min-w-0">
      <div className="mb-1.5 flex h-6 min-w-0 items-center gap-0.5">
        <h3 className="flex min-w-0">
          <ColumnMenu facet={facet} tools={tools}>
            <button
              aria-label={`${facetTitle(facet)} column options`}
              className="flex min-w-0 items-center gap-0.5 rounded text-[11px] font-semibold uppercase tracking-[0.06em] text-zinc-500 hover:text-zinc-200"
              title={`${facetTitle(facet)}: order, pin or hide`}
              type="button"
            >
              <span className="truncate">{facetTitle(facet)}</span>
              <ChevronDown className="h-3 w-3 shrink-0" />
            </button>
          </ColumnMenu>
        </h3>
        <ColumnToggles facet={facet} revealOnHover tools={tools} />
      </div>
      <FacetBody
        facet={facet}
        onCustomRange={onCustomRange}
        order={tools.order}
        query={query}
        setQuery={setQuery}
      />
    </div>
  );
}

function FacetBody(props: {
  facet: Facet;
  onCustomRange: () => void;
  order: FacetValueOrder;
  query: FocusQuery;
  roomy?: boolean;
  setQuery: (query: FocusQuery) => void;
}) {
  return isYearFacet(props.facet.key) ? (
    <YearValues {...props} />
  ) : (
    <FacetValues {...props} />
  );
}

/**
 * A column's values, most frequent first, capped at six rows. "View more"
 * opens the column in place: it loads the rest of the values and, for long
 * lists, a search field that asks the server, so it scales to libraries far
 * larger than the rows it shows.
 */
function FacetValues({
  facet,
  order,
  query,
  roomy,
  setQuery,
}: {
  facet: Facet;
  order: FacetValueOrder;
  query: FocusQuery;
  roomy?: boolean;
  setQuery: (query: FocusQuery) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [search, setSearch] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(search.trim()), 200);
    return () => window.clearTimeout(timer);
  }, [search]);
  const base = facetBaseQuery(query);
  const full = useQuery({
    enabled: expanded,
    queryKey: [
      "library",
      "focus",
      "facet-values",
      facet.key,
      facet.role ?? null,
      debounced,
      facetQueryKey(base),
    ],
    queryFn: () =>
      getFocusFacets(base, {
        only: [facet.key],
        search: debounced,
        limit: EXPANDED_VALUES,
      }),
    placeholderData: keepPreviousData,
    staleTime: 30_000,
  });
  const loaded =
    full.data?.facets.find(
      (candidate) =>
        candidate.key === facet.key &&
        (candidate.role ?? null) === (facet.role ?? null),
    ) ?? null;
  const searchable = expanded && facet.total > COLUMN_VALUES * 2;
  const source = expanded && loaded ? loaded : facet;
  const { values, more } = orderedFacetValues(
    {
      ...source,
      values: source.values.filter(
        (value) =>
          value.count > 0 || facetValueState(query, facet, value) !== null,
      ),
    },
    query,
    { cap: expanded ? undefined : COLUMN_VALUES, order },
  );
  const inputRef = useRef<HTMLInputElement>(null);

  return (
    <div className="min-w-0">
      {searchable && (
        <label className="mb-1.5 flex h-8 items-center gap-1.5 rounded-md border border-white/10 bg-black/40 px-2 text-sm focus-within:border-white/25">
          <Search className="h-3.5 w-3.5 shrink-0 text-zinc-500" />
          <input
            aria-label={`Search ${facetTitle(facet)}`}
            autoFocus
            className="min-w-0 flex-1 bg-transparent text-zinc-100 outline-none placeholder:text-zinc-600"
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search"
            ref={inputRef}
            value={search}
          />
          {search && (
            <button
              aria-label="Clear search"
              className="text-zinc-500 hover:text-white"
              onClick={() => {
                setSearch("");
                inputRef.current?.focus();
              }}
              type="button"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </label>
      )}
      <ul
        className={cn(
          "space-y-0.5",
          expanded && "-mr-2 max-h-64 overflow-y-auto overscroll-contain pr-2",
        )}
      >
        {values.map((value) => (
          <li key={value.value}>
            <FacetValueButton
              count={value.count}
              label={value.label}
              onClick={() => setQuery(toggleFacetValue(query, facet, value))}
              roomy={roomy}
              state={facetValueState(query, facet, value)}
            />
          </li>
        ))}
        {expanded && full.isSuccess && values.length === 0 && (
          <li className="py-3 text-sm text-zinc-500">No matches</li>
        )}
      </ul>
      {expanded && loaded && more > 0 && (
        <p className="mt-1 text-xs text-zinc-600">
          {more.toLocaleString("en-US")} more. Search to find them.
        </p>
      )}
      {!expanded && more > 0 && (
        <button
          className="mt-1.5 text-sm text-zinc-500 underline-offset-4 hover:text-white hover:underline"
          onClick={() => setExpanded(true)}
          type="button"
        >
          View more{" "}
          <span className="tabular-nums text-zinc-600">
            ({more.toLocaleString("en-US")})
          </span>
        </button>
      )}
      {expanded && (
        <button
          className="mt-1.5 text-sm text-zinc-500 underline-offset-4 hover:text-white hover:underline"
          onClick={() => {
            setExpanded(false);
            setSearch("");
          }}
          type="button"
        >
          Show less
        </button>
      )}
    </div>
  );
}

function YearValues({
  facet,
  onCustomRange,
  query,
  roomy,
  setQuery,
}: {
  facet: Facet;
  onCustomRange: () => void;
  query: FocusQuery;
  roomy?: boolean;
  setQuery: (query: FocusQuery) => void;
}) {
  const yearKey = facet.key === "composed" ? "composed" : "year";
  const decades = decadesFromYears(facet.values);
  const max = Math.max(1, ...facet.values.map((value) => value.count));
  const years = facet.values.map((value) => Number(value.value));
  const firstYear = Math.min(...years);
  const yearSpan = Math.max(1, Math.max(...years) - firstYear);
  return (
    <div className="min-w-0">
      <button
        aria-label="Choose a custom year range"
        className="relative mb-2 block h-8 w-full rounded opacity-80 hover:opacity-100"
        onClick={onCustomRange}
        title="Choose a custom range"
        type="button"
      >
        <span className="absolute inset-x-0 bottom-0 h-px bg-white/10" />
        {facet.values.map((value) => {
          const year = Number(value.value);
          return (
            <span
              className="absolute bottom-0 w-[3px] -translate-x-1/2 rounded-t-sm bg-zinc-400"
              key={value.value}
              style={{
                left: `${((year - firstYear) / yearSpan) * 96 + 2}%`,
                height: `${Math.max(18, (value.count / max) * 100)}%`,
              }}
              title={`${value.label} · ${value.count}`}
            />
          );
        })}
      </button>
      <ul className="space-y-0.5">
        {decades.slice(-COLUMN_VALUES).map((bucket) => (
          <li key={bucket.decade}>
            <FacetValueButton
              count={bucket.count}
              label={`${bucket.decade}s`}
              onClick={() =>
                setQuery(
                  cycleFilter(query, decadeCriterion(bucket.decade, yearKey)),
                )
              }
              roomy={roomy}
              state={filterState(
                query,
                decadeCriterion(bucket.decade, yearKey),
              )}
            />
          </li>
        ))}
      </ul>
      <button
        className="mt-1.5 text-sm text-zinc-500 underline-offset-4 hover:text-white hover:underline"
        onClick={onCustomRange}
        type="button"
      >
        {decades.length > COLUMN_VALUES ? "View more" : "Custom range"}
      </button>
    </div>
  );
}

/** Custom year range for Release date and Composition date. */
function YearRangeDialog({
  facet,
  onOpenChange,
  query,
  setQuery,
}: {
  facet: Facet | null;
  onOpenChange: (open: boolean) => void;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  // A range facet lists every year the other filters allow, so the range
  // can be widened as well as narrowed.
  const yearFacet = facet && isYearFacet(facet.key) ? facet.key : null;
  const base = {
    ...facetBaseQuery(query),
    filters: facetBaseQuery(query).filters.filter(
      (filter) => filter.facet !== yearFacet,
    ),
  };
  const values = useQuery({
    enabled: Boolean(yearFacet),
    queryKey: [
      "library",
      "focus",
      "facet-values",
      yearFacet,
      facetQueryKey(base),
    ],
    queryFn: () =>
      getFocusFacets(base, {
        only: yearFacet ? [yearFacet] : [],
        limit: EXPANDED_VALUES,
      }),
    placeholderData: keepPreviousData,
  });
  const loaded =
    values.data?.facets.find((candidate) => candidate.key === yearFacet) ??
    null;

  return (
    <Dialog onOpenChange={onOpenChange} open={Boolean(yearFacet)}>
      <DialogContent aria-describedby={undefined} className="max-w-md">
        <DialogHeader>
          <DialogTitle>{facet ? facetTitle(facet) : ""}</DialogTitle>
        </DialogHeader>
        {facet && yearFacet && (
          <YearRange
            facet={loaded ?? facet}
            facetKey={yearFacet}
            key={loaded ? "loaded" : "initial"}
            query={query}
            setQuery={setQuery}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function YearRange({
  facet,
  facetKey,
  query,
  setQuery,
}: {
  facet: Facet;
  facetKey: "year" | "composed";
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const years = facet.values.map((value) => Number(value.value));
  const first = Math.min(...years);
  const last = Math.max(...years);
  const current = query.filters.find((filter) => filter.facet === facetKey);
  const [from, setFrom] = useState(
    String(current && "from" in current && current.from ? current.from : first),
  );
  const [to, setTo] = useState(
    String(current && "to" in current && current.to ? current.to : last),
  );
  const max = Math.max(1, ...facet.values.map((value) => value.count));
  const fromYear = Number(from);
  const toYear = Number(to);
  const valid =
    Number.isInteger(fromYear) &&
    Number.isInteger(toYear) &&
    fromYear > 0 &&
    fromYear <= toYear &&
    toYear <= 9999;

  return (
    <div className="grid gap-5">
      <div className="relative h-24 w-full border-b border-white/10">
        {facet.values.map((value) => {
          const year = Number(value.value);
          const inside = valid && year >= fromYear && year <= toYear;
          return (
            <button
              aria-label={`${value.label}: ${value.count}`}
              className={cn(
                "absolute bottom-0 w-2 -translate-x-1/2 rounded-t-sm transition-colors",
                inside ? "bg-white" : "bg-zinc-700 hover:bg-zinc-500",
              )}
              key={value.value}
              onClick={() => {
                setFrom(value.value);
                setTo(value.value);
              }}
              style={{
                left: `${((year - first) / Math.max(1, last - first)) * 94 + 3}%`,
                height: `${Math.max(8, (value.count / max) * 100)}%`,
              }}
              title={`${value.label} · ${value.count}`}
              type="button"
            />
          );
        })}
      </div>
      <div className="-mt-3 flex justify-between text-[11px] tabular-nums text-zinc-600">
        <span>{first}</span>
        <span>{last}</span>
      </div>
      <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-3">
        <input
          aria-label="From year"
          className="h-10 w-full min-w-0 rounded-md border border-zinc-800 bg-black px-3 text-sm tabular-nums text-zinc-100 outline-none focus:border-zinc-600"
          inputMode="numeric"
          onChange={(event) => setFrom(event.target.value)}
          value={from}
        />
        <span className="text-zinc-500">to</span>
        <input
          aria-label="To year"
          className="h-10 w-full min-w-0 rounded-md border border-zinc-800 bg-black px-3 text-sm tabular-nums text-zinc-100 outline-none focus:border-zinc-600"
          inputMode="numeric"
          onChange={(event) => setTo(event.target.value)}
          value={to}
        />
      </div>
      <div className="flex flex-wrap gap-2">
        {decadesFromYears(facet.values).map((bucket) => (
          <button
            className="rounded-full border border-white/12 px-3 py-1 text-xs text-zinc-300 hover:border-white/30 hover:text-white"
            key={bucket.decade}
            onClick={() => {
              setFrom(String(bucket.decade));
              setTo(String(bucket.decade + 9));
            }}
            type="button"
          >
            {bucket.decade}s · {bucket.count}
          </button>
        ))}
      </div>
      <div className="flex justify-end gap-2">
        {current && (
          <button
            className="h-10 rounded-md px-4 text-sm text-zinc-400 hover:text-white"
            onClick={() =>
              setQuery({
                ...query,
                filters: query.filters.filter(
                  (filter) => filter.facet !== facetKey,
                ),
              })
            }
            type="button"
          >
            Clear
          </button>
        )}
        <button
          className="h-10 rounded-md bg-white px-4 text-sm font-medium text-black hover:bg-zinc-200 disabled:opacity-50"
          disabled={!valid}
          onClick={() =>
            setQuery(
              setFilter(query, {
                facet: facetKey,
                from: fromYear,
                to: toYear,
              }),
            )
          }
          type="button"
        >
          Focus on{" "}
          {valid
            ? fromYear === toYear
              ? fromYear
              : `${fromYear}–${toYear}`
            : "years"}
        </button>
      </div>
    </div>
  );
}
