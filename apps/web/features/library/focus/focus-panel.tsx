"use client";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import {
  cycleFilter,
  decadeCriterion,
  decadesFromYears,
  facetBaseQuery,
  facetTitle,
  facetValueState,
  filterState,
  getFocusFacets,
  isYearFacet,
  setFilter,
  toggleFacetValue,
  visibleFacets,
  type Facet,
  type FacetValue,
  type FilterState,
  type FocusQuery,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { ChevronLeft, ChevronRight, Minus, Plus, Search } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  anchoredScrollLeft,
  facetQueryKey,
  scrollAnchor,
  type ColumnBox,
} from "./focus-state";

const COLUMN_VALUES = 6;

interface ScrollAnchor {
  /** The first column at least partly visible. */
  key: string;
  /** Its left edge relative to the visible area, in pixels. */
  offset: number;
}

/**
 * Where each panel was scrolled, by browser and scope, so a click that
 * changes, adds or removes columns (or remounts the panel) keeps the same
 * columns under the pointer instead of jumping.
 */
const scrollMemory = new Map<string, ScrollAnchor>();

function columnKey(facet: Facet) {
  return `${facet.key}:${facet.role ?? ""}`;
}

function columnBoxes(node: HTMLElement): ColumnBox[] {
  return Array.from(
    node.querySelectorAll<HTMLElement>("[data-focus-column]"),
  ).map((column) => ({
    key: column.dataset.focusColumn ?? "",
    left: column.offsetLeft,
    width: column.offsetWidth,
  }));
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
  const [more, setMore] = useState<Facet | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: true, end: true });
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
  const memoryKey = `${query.entity}:${JSON.stringify(query.scope ?? [])}`;

  const rememberAnchor = useCallback(() => {
    const node = scroller.current;
    if (!node) return;
    const anchor = scrollAnchor(columnBoxes(node), node.scrollLeft);
    if (anchor) scrollMemory.set(memoryKey, anchor);
  }, [memoryKey]);

  // Restore the anchor column's position after columns change or the panel
  // mounts, without the smooth scrolling used by the arrows.
  useLayoutEffect(() => {
    const node = scroller.current;
    const anchor = scrollMemory.get(memoryKey);
    if (!node || !anchor) return;
    const left = anchoredScrollLeft(columnBoxes(node), anchor);
    if (left === null || Math.abs(node.scrollLeft - left) < 1) return;
    const behavior = node.style.scrollBehavior;
    node.style.scrollBehavior = "auto";
    node.scrollLeft = left;
    node.style.scrollBehavior = behavior;
  }, [columns, memoryKey]);

  useEffect(() => {
    const node = scroller.current;
    if (!node) return;
    const update = () =>
      setEdges({
        start: node.scrollLeft <= 4,
        end: node.scrollLeft + node.clientWidth >= node.scrollWidth - 4,
      });
    const onScroll = () => {
      update();
      rememberAnchor();
    };
    update();
    node.addEventListener("scroll", onScroll, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(node);
    return () => {
      node.removeEventListener("scroll", onScroll);
      observer.disconnect();
    };
  }, [columns.length, rememberAnchor]);

  const page = (direction: 1 | -1) => {
    const node = scroller.current;
    if (!node) return;
    node.scrollBy({
      left: direction * node.clientWidth * 0.8,
      behavior: "smooth",
    });
  };

  return (
    <section
      aria-label="Focus"
      className="relative rounded-2xl border border-white/10 bg-white/[0.025]"
      id="library-focus-panel"
    >
      <div className="flex items-center justify-between gap-3 border-b border-white/6 px-5 py-2.5 text-xs text-zinc-500">
        <p>Click to focus, click again to exclude.</p>
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
                  "rounded-full px-2.5 py-0.5 text-xs transition-colors",
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
                Match {mode}
              </button>
            );
          })}
        </div>
      </div>

      {facets.isPending && !facets.data ? (
        <div className="grid grid-flow-col auto-cols-[minmax(10.5rem,1fr)] gap-6 px-5 py-5">
          {Array.from({ length: 5 }, (_, index) => (
            <div className="space-y-3" key={index}>
              <div className="h-3 w-20 rounded bg-white/8" />
              {Array.from({ length: 5 }, (__, row) => (
                <div className="h-3 w-28 rounded bg-white/5" key={row} />
              ))}
            </div>
          ))}
        </div>
      ) : columns.length === 0 ? (
        <p className="px-5 py-8 text-sm text-zinc-500">
          {facets.isError
            ? "Focus options could not be loaded."
            : "Nothing left to focus on. Remove a filter to see more options."}
        </p>
      ) : (
        <div className="relative">
          <div
            className={cn(
              "grid grid-flow-col auto-cols-[minmax(10.5rem,1fr)] gap-6 overflow-x-auto scroll-smooth px-5 py-5 scrollbar-none [&::-webkit-scrollbar]:hidden",
              facets.isFetching && "opacity-80",
            )}
            ref={scroller}
          >
            {columns.map((facet) => (
              <div
                className="min-w-0"
                data-focus-column={columnKey(facet)}
                key={columnKey(facet)}
                // A click inside a column pins that column in place.
                onPointerDownCapture={rememberAnchor}
              >
                <FacetColumn
                  facet={facet}
                  onMore={() => setMore(facet)}
                  query={query}
                  setQuery={setQuery}
                />
              </div>
            ))}
          </div>
          {!edges.start && (
            <button
              aria-label="Previous focus columns"
              className="absolute left-1 top-1/2 grid h-8 w-8 -translate-y-1/2 place-items-center rounded-full bg-black/80 text-zinc-300 shadow-lg hover:text-white"
              onClick={() => page(-1)}
              type="button"
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
          )}
          {!edges.end && (
            <button
              aria-label="More focus columns"
              className="absolute right-1 top-1/2 grid h-8 w-8 -translate-y-1/2 place-items-center rounded-full bg-black/80 text-zinc-300 shadow-lg hover:text-white"
              onClick={() => page(1)}
              type="button"
            >
              <ChevronRight className="h-4 w-4" />
            </button>
          )}
        </div>
      )}
      <FacetMoreDialog
        facet={more}
        onOpenChange={(open) => !open && setMore(null)}
        query={query}
        setQuery={setQuery}
      />
    </section>
  );
}

function FacetValueButton({
  label,
  count,
  onClick,
  state,
}: {
  label: string;
  count: number;
  onClick: () => void;
  state: FilterState;
}) {
  return (
    <button
      aria-pressed={state !== null}
      className={cn(
        "group flex w-full min-w-0 items-center gap-1.5 rounded-md py-1 text-left text-sm transition-colors",
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
  onMore,
  query,
  setQuery,
}: {
  facet: Facet;
  onMore: () => void;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  if (isYearFacet(facet.key)) {
    return (
      <YearColumn
        facet={facet}
        onMore={onMore}
        query={query}
        setQuery={setQuery}
      />
    );
  }
  const values = facet.values.slice(0, COLUMN_VALUES);
  return (
    <div className="min-w-0">
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-zinc-500">
        {facetTitle(facet)}
      </h3>
      <ul className="space-y-0.5">
        {values.map((value) => (
          <li key={value.value}>
            <FacetValueButton
              count={value.count}
              label={value.label}
              onClick={() => setQuery(toggleFacetValue(query, facet, value))}
              state={facetValueState(query, facet, value)}
            />
          </li>
        ))}
      </ul>
      {facet.total > values.length && (
        <button
          className="mt-2 text-sm text-zinc-500 underline-offset-4 hover:text-white hover:underline"
          onClick={onMore}
          type="button"
        >
          View more
        </button>
      )}
    </div>
  );
}

function YearColumn({
  facet,
  onMore,
  query,
  setQuery,
}: {
  facet: Facet;
  onMore: () => void;
  query: FocusQuery;
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
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-zinc-500">
        {facetTitle(facet)}
      </h3>
      <button
        aria-label="Choose a custom year range"
        className="relative mb-2 block h-8 w-full rounded opacity-80 hover:opacity-100"
        onClick={onMore}
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
              state={filterState(
                query,
                decadeCriterion(bucket.decade, yearKey),
              )}
            />
          </li>
        ))}
      </ul>
      <button
        className="mt-2 text-sm text-zinc-500 underline-offset-4 hover:text-white hover:underline"
        onClick={onMore}
        type="button"
      >
        {decades.length > COLUMN_VALUES ? "View more" : "Custom range"}
      </button>
    </div>
  );
}

function FacetMoreDialog({
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
  const [search, setSearch] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(search), 200);
    return () => window.clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    setSearch("");
    setDebounced("");
  }, [facet?.key, facet?.role]);

  // A range facet lists every year the other filters allow, so the range
  // can be widened as well as narrowed.
  const yearFacet = facet && isYearFacet(facet.key) ? facet.key : null;
  const base = yearFacet
    ? {
        ...facetBaseQuery(query),
        filters: facetBaseQuery(query).filters.filter(
          (filter) => filter.facet !== yearFacet,
        ),
      }
    : facetBaseQuery(query);
  const values = useQuery({
    enabled: Boolean(facet),
    queryKey: [
      "library",
      "focus",
      "facet-values",
      facet?.key,
      facet?.role,
      debounced,
      facetQueryKey(base),
    ],
    queryFn: () =>
      getFocusFacets(base, {
        only: facet ? [facet.key] : [],
        search: yearFacet ? undefined : debounced,
        limit: 200,
      }),
    placeholderData: keepPreviousData,
  });
  const loaded =
    values.data?.facets.find(
      (candidate) =>
        candidate.key === facet?.key &&
        (candidate.role ?? null) === (facet?.role ?? null),
    ) ?? null;

  return (
    <Dialog onOpenChange={onOpenChange} open={Boolean(facet)}>
      <DialogContent aria-describedby={undefined} className="max-w-md">
        <DialogHeader>
          <DialogTitle>{facet ? facetTitle(facet) : ""}</DialogTitle>
        </DialogHeader>
        {facet && yearFacet ? (
          <YearRange
            facet={loaded ?? facet}
            facetKey={yearFacet}
            key={loaded ? "loaded" : "initial"}
            query={query}
            setQuery={setQuery}
          />
        ) : (
          <>
            <label className="flex h-10 items-center gap-2 rounded-md border border-zinc-800 bg-black px-3 text-sm focus-within:border-zinc-600">
              <Search className="h-4 w-4 text-zinc-500" />
              <input
                aria-label="Filter values"
                autoFocus
                className="min-w-0 flex-1 bg-transparent text-zinc-100 outline-none placeholder:text-zinc-600"
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Filter"
                value={search}
              />
            </label>
            <ul className="-mx-1 max-h-[55vh] space-y-0.5 overflow-y-auto px-1">
              {(loaded?.values ?? [])
                .filter((value: FacetValue) => value.count > 0)
                .map((value: FacetValue) =>
                  facet ? (
                    <li key={value.value}>
                      <FacetValueButton
                        count={value.count}
                        label={value.label}
                        onClick={() =>
                          setQuery(toggleFacetValue(query, facet, value))
                        }
                        state={facetValueState(query, facet, value)}
                      />
                    </li>
                  ) : null,
                )}
              {values.isSuccess && !loaded?.values.length && (
                <li className="py-6 text-center text-sm text-zinc-500">
                  No matches
                </li>
              )}
            </ul>
          </>
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
