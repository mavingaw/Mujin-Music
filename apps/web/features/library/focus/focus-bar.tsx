"use client";

import {
  DropdownMenu,
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
  SORT_OPTIONS,
  criterionLabel,
  cycleFilter,
  effectiveDirection,
  effectiveSort,
  filterKey,
  filterState,
  labelCarriesExclusion,
  removeFilter,
  toggleExclude,
  type FocusQuery,
  type FocusSort,
} from "@parson/music-sdk";
import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  ChevronDown,
  Heart,
  ListMusic,
  Minus,
  Play,
  Plus,
  Search,
  Shuffle,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import SavedFocusMenu from "./saved-focus-menu";

export default function FocusBar({
  bookmarks = true,
  canPlay,
  extraActions,
  onCreateSmartPlaylist,
  onPlay,
  onTogglePanel,
  panelOpen,
  query,
  setQuery,
}: {
  /** Scoped browsers (a composer, an album) have no bookmarks. */
  bookmarks?: boolean;
  canPlay: boolean;
  /** More actions after "Create smart playlist" (tagging). */
  extraActions?: ReactNode;
  onCreateSmartPlaylist: () => void;
  onPlay: (shuffle: boolean) => void;
  onTogglePanel: () => void;
  panelOpen: boolean;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const liked = filterState(query, { facet: "liked" });
  const focused = query.filters.length > 0 || Boolean(query.text?.trim());
  const visibleFilters = query.filters.filter(
    (filter) => filter.facet !== "liked",
  );

  return (
    <div className="space-y-3">
      <div
        className="flex flex-wrap items-center gap-2 [&>*]:shrink-0"
        role="toolbar"
      >
        <button
          aria-controls="library-focus-panel"
          aria-expanded={panelOpen}
          className={cn(
            "flex h-9 items-center gap-1.5 rounded-full border px-3.5 text-sm font-medium transition-colors",
            panelOpen
              ? "border-white/40 bg-white/10 text-white"
              : "border-white/12 text-zinc-300 hover:border-white/25 hover:text-white",
          )}
          onClick={onTogglePanel}
          type="button"
        >
          <ChevronDown
            className={cn(
              "h-4 w-4 transition-transform",
              panelOpen && "rotate-180",
            )}
          />
          Focus
          {query.filters.length > 0 && (
            <span className="ml-0.5 rounded-full bg-white px-1.5 text-[0.6875rem] font-semibold leading-4 text-black tabular-nums">
              {query.filters.length}
            </span>
          )}
        </button>
        <button
          aria-label={
            liked === "include"
              ? "Showing liked only. Click to hide liked"
              : liked === "exclude"
                ? "Hiding liked. Click to clear"
                : "Show liked only"
          }
          aria-pressed={liked !== null}
          className={cn(
            "relative grid h-9 w-9 place-items-center rounded-full border transition-colors",
            liked === "include" &&
              "border-rose-400/50 bg-rose-500/15 text-rose-300",
            liked === "exclude" && "border-zinc-500 text-zinc-400",
            liked === null &&
              "border-white/12 text-zinc-400 hover:border-white/25 hover:text-white",
          )}
          onClick={() => setQuery(cycleFilter(query, { facet: "liked" }))}
          title="Liked"
          type="button"
        >
          <Heart
            className={cn(
              "h-4 w-4",
              liked === "include" && "fill-current",
              liked === "exclude" && "opacity-60",
            )}
          />
          {liked === "exclude" && (
            <span className="pointer-events-none absolute h-5 w-px rotate-45 bg-zinc-400" />
          )}
        </button>

        {focused && (
          <button
            aria-label="Create smart playlist"
            className="flex h-9 items-center gap-1.5 rounded-full px-2 text-sm text-zinc-300 hover:text-white"
            onClick={onCreateSmartPlaylist}
            type="button"
          >
            <ListMusic className="h-4 w-4" />
            <span className="hidden md:inline">Create smart playlist</span>
          </button>
        )}
        {extraActions}

        <div className="ml-auto flex items-center gap-1 [&>*]:shrink-0">
          {canPlay && (
            <>
              <button
                aria-label="Play focused music"
                className="mj-primary grid h-9 w-9 place-items-center rounded-full"
                onClick={() => onPlay(false)}
                title="Play"
                type="button"
              >
                <Play className="h-4 w-4 fill-current" />
              </button>
              <button
                aria-label="Shuffle focused music"
                className="grid h-9 w-9 place-items-center rounded-full text-zinc-400 hover:bg-white/8 hover:text-white"
                onClick={() => onPlay(true)}
                title="Shuffle"
                type="button"
              >
                <Shuffle className="h-4 w-4" />
              </button>
            </>
          )}
          <FocusTextFilter
            onChange={(text) => setQuery({ ...query, text })}
            value={query.text ?? ""}
          />
          <SortMenu query={query} setQuery={setQuery} />
          {bookmarks && <SavedFocusMenu query={query} setQuery={setQuery} />}
        </div>
      </div>
      {visibleFilters.length > 0 && (
        <div
          aria-label="Active focus filters"
          className="flex flex-wrap items-center gap-2"
        >
          {visibleFilters.map((filter) => {
            const key = filterKey(filter);
            const excluded = Boolean(filter.exclude);
            const label = criterionLabel(filter);
            return (
              <span
                className={cn(
                  "flex h-9 max-w-64 items-center rounded-full border pl-1 pr-1 text-sm",
                  excluded
                    ? "border-rose-400/35 bg-rose-500/8 text-rose-100"
                    : "border-white/15 bg-white/[0.07] text-white",
                )}
                key={key}
              >
                {!labelCarriesExclusion(filter) && (
                  <button
                    aria-label={
                      excluded ? `Include ${label} instead` : `Exclude ${label}`
                    }
                    className="grid h-7 w-7 shrink-0 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
                    onClick={() => setQuery(toggleExclude(query, key))}
                    title={
                      excluded
                        ? "Excluded. Click to include"
                        : "Click to exclude"
                    }
                    type="button"
                  >
                    {excluded ? (
                      <Minus className="h-3.5 w-3.5" />
                    ) : (
                      <Plus className="h-3.5 w-3.5" />
                    )}
                  </button>
                )}
                <span
                  className={cn(
                    "truncate px-1",
                    labelCarriesExclusion(filter) && "pl-2.5",
                    excluded &&
                      !labelCarriesExclusion(filter) &&
                      "line-through decoration-rose-300/70",
                  )}
                  title={label}
                >
                  {label}
                </span>
                <button
                  aria-label={`Remove ${label}`}
                  className="grid h-7 w-7 shrink-0 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
                  onClick={() => setQuery(removeFilter(query, key))}
                  type="button"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </span>
            );
          })}

          {query.filters.length > 1 && (
            <button
              className="h-9 rounded-full px-2 text-sm text-zinc-500 hover:text-white"
              onClick={() =>
                setQuery({ ...query, filters: [], match: undefined })
              }
              type="button"
            >
              Clear all
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function FocusTextFilter({
  onChange,
  value,
}: {
  onChange: (value: string) => void;
  value: string;
}) {
  const [draft, setDraft] = useState(value);
  const [open, setOpen] = useState(Boolean(value));
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    setDraft(value);
    if (value) setOpen(true);
  }, [value]);

  useEffect(() => {
    if (draft === value) return;
    const timer = window.setTimeout(() => onChangeRef.current(draft), 250);
    return () => window.clearTimeout(timer);
  }, [draft, value]);

  if (!open) {
    return (
      <button
        aria-label="Filter"
        className="flex h-9 items-center gap-1.5 rounded-full px-2.5 text-sm text-zinc-400 hover:bg-white/8 hover:text-white"
        onClick={() => setOpen(true)}
        type="button"
      >
        <Search className="h-4 w-4" />
        <span className="hidden sm:inline">Filter</span>
      </button>
    );
  }
  return (
    <label className="flex h-9 w-44 items-center gap-1.5 rounded-full border border-white/15 bg-white/[0.04] px-3 text-sm focus-within:border-white/35">
      <Search className="h-4 w-4 shrink-0 text-zinc-500" />
      <input
        aria-label="Filter by title or artist"
        autoFocus={!value}
        className="min-w-0 flex-1 bg-transparent text-zinc-100 outline-none placeholder:text-zinc-600"
        maxLength={200}
        onBlur={() => {
          if (!draft.trim()) setOpen(false);
        }}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setDraft("");
            onChange("");
            setOpen(false);
          }
        }}
        placeholder="Filter"
        value={draft}
      />
      {draft && (
        <button
          aria-label="Clear filter"
          className="text-zinc-500 hover:text-white"
          onClick={() => {
            setDraft("");
            onChange("");
          }}
          type="button"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </label>
  );
}

function SortMenu({
  query,
  setQuery,
}: {
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const sort = effectiveSort(query);
  const direction = effectiveDirection(query);
  const options = SORT_OPTIONS[query.entity];
  const current = options.find((option) => option.value === sort);
  const DirectionIcon =
    direction === "asc" ? ArrowUpNarrowWide : ArrowDownWideNarrow;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className="flex h-9 items-center gap-1.5 rounded-full px-2.5 text-sm text-zinc-400 hover:bg-white/8 hover:text-white"
          type="button"
        >
          {sort !== "random" && <DirectionIcon className="h-4 w-4" />}
          <span className="hidden sm:inline">
            By {current?.label.toLowerCase()}
          </span>
          <ChevronDown className="h-3.5 w-3.5" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-52">
        <DropdownMenuLabel>Sort by</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          onValueChange={(value) => {
            const next = value as FocusSort;
            setQuery({
              ...query,
              sort: next,
              direction: null,
              seed:
                next === "random"
                  ? Math.floor(Math.random() * 1_000_000) + 1
                  : null,
            });
          }}
          value={sort}
        >
          {options.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
        {sort !== "random" && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={() =>
                setQuery({
                  ...query,
                  direction: direction === "asc" ? "desc" : "asc",
                })
              }
            >
              <DirectionIcon />
              {direction === "asc" ? "Ascending" : "Descending"}
            </DropdownMenuItem>
          </>
        )}
        {sort === "random" && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={() =>
                setQuery({
                  ...query,
                  seed: Math.floor(Math.random() * 1_000_000) + 1,
                })
              }
            >
              <Shuffle />
              Reshuffle
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
