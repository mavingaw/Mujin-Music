"use client";

import {
  STREAMING_PROVIDER_NAMES,
  type CatalogFacets,
  type CatalogQuality,
  type CatalogSort,
  type CatalogSource,
  type StreamingProviderId,
} from "@parson/music-sdk";
import { HardDrive, Layers, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { ProviderMark } from "./source-badge";
import {
  activeFocusCount,
  catalogSorts,
  decadeLabel,
  describeFocus,
  emptyFocus,
  qualityLabels,
  splitFacets,
  type LibraryFocus,
} from "./library-focus";

const MAX_GENRE_CHIPS = 8;

function Chip({
  active,
  count,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  count?: number;
  icon?: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-pressed={active}
      className={cn(
        "inline-flex h-8 shrink-0 items-center gap-2 rounded-full border px-3 text-[0.8125rem] font-medium transition-colors",
        active
          ? "border-white bg-white text-black"
          : "border-white/12 text-zinc-300 hover:border-white/30 hover:text-white",
        !active && count === 0 && "opacity-45",
      )}
      onClick={onClick}
      type="button"
    >
      {icon}
      {label}
      {count !== undefined && (
        <span
          className={cn(
            "tabular-nums text-[0.6875rem]",
            active ? "text-black/55" : "text-zinc-500",
          )}
        >
          {count}
        </span>
      )}
    </button>
  );
}

function FacetRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 items-center gap-3">
      <span className="w-16 shrink-0 text-[0.6875rem] font-semibold uppercase tracking-[0.08em] text-zinc-500">
        {label}
      </span>
      <div
        aria-label={label}
        className="flex min-w-0 flex-1 gap-2 overflow-x-auto pb-0.5 scrollbar-none [&::-webkit-scrollbar]:hidden"
        role="group"
      >
        {children}
      </div>
    </div>
  );
}

const countOf = <T,>(
  facets: { value: T; count: number }[] | undefined,
  value: T,
) => facets?.find((facet) => facet.value === value)?.count ?? 0;

/**
 * Roon's Focus for albums: storage location, genre, decade and quality
 * chips that combine. Counts say how many albums each choice would show
 * given the other filters.
 */
export function LibraryFocusBar({
  focus,
  facets,
  available,
  total,
  onChange,
  sort,
  onSortChange,
}: {
  focus: LibraryFocus;
  facets: CatalogFacets | undefined;
  available: StreamingProviderId[];
  total: number | undefined;
  onChange: (focus: LibraryFocus) => void;
  sort?: CatalogSort;
  onSortChange?: (sort: CatalogSort) => void;
}) {
  const set = (patch: Partial<LibraryFocus>) =>
    onChange({ ...focus, ...patch });
  const streamingSources = (["tidal", "qobuz"] as const).filter(
    (provider) =>
      available.includes(provider) ||
      countOf(facets?.sources, provider) > 0 ||
      focus.source === provider,
  );
  const allCount = facets?.sources.reduce((sum, facet) => sum + facet.count, 0);
  const genres = splitFacets(
    facets?.genres ?? [],
    focus.genre,
    MAX_GENRE_CHIPS,
  );
  const decades = splitFacets(facets?.decades ?? [], focus.decade, 20).shown;
  const qualities = (
    ["hi_res", "lossless", "lossy"] as CatalogQuality[]
  ).filter(
    (quality) =>
      countOf(facets?.qualities, quality) > 0 || focus.quality === quality,
  );
  const summary = describeFocus(focus, STREAMING_PROVIDER_NAMES);
  const sourceChip = (value: CatalogSource) =>
    value === "local"
      ? {
          label: "Local files",
          icon: <HardDrive className="h-3.5 w-3.5" />,
        }
      : {
          label: `${STREAMING_PROVIDER_NAMES[value]} library`,
          icon: (
            <ProviderMark
              className="h-3.5 w-3.5 text-[0.5rem]"
              onLight={focus.source === value}
              provider={value}
            />
          ),
        };

  return (
    <section
      aria-label="Focus"
      className="space-y-2.5 rounded-xl border border-white/8 bg-white/[0.02] p-3 sm:p-4"
    >
      {sort && onSortChange && (
        <FacetRow label="Sort">
          <select
            aria-label="Sort albums"
            className="h-8 shrink-0 rounded-full border border-white/12 bg-zinc-950 px-3 text-[0.8125rem] text-zinc-200 hover:border-white/30"
            onChange={(event) =>
              onSortChange(event.target.value as CatalogSort)
            }
            value={sort}
          >
            {catalogSorts.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </FacetRow>
      )}
      {(streamingSources.length > 0 || focus.source !== "all") && (
        <FacetRow label="Source">
          <Chip
            active={focus.source === "all"}
            count={allCount}
            icon={<Layers className="h-3.5 w-3.5" />}
            label="All sources"
            onClick={() => set({ source: "all" })}
          />
          {(["local", ...streamingSources] as CatalogSource[]).map((value) => (
            <Chip
              active={focus.source === value}
              count={facets ? countOf(facets.sources, value) : undefined}
              key={value}
              onClick={() =>
                set({ source: focus.source === value ? "all" : value })
              }
              {...sourceChip(value)}
            />
          ))}
        </FacetRow>
      )}
      {genres.shown.length > 0 && (
        <FacetRow label="Genre">
          {genres.shown.map((facet) => (
            <Chip
              active={focus.genre === facet.value}
              count={facet.count}
              key={facet.value}
              label={facet.value}
              onClick={() =>
                set({ genre: focus.genre === facet.value ? null : facet.value })
              }
            />
          ))}
          {genres.more.length > 0 && (
            <select
              aria-label="More genres"
              className="h-8 shrink-0 rounded-full border border-white/12 bg-transparent px-3 text-[0.8125rem] text-zinc-300 hover:border-white/30"
              onChange={(event) =>
                event.target.value && set({ genre: event.target.value })
              }
              value=""
            >
              <option value="">More genres…</option>
              {genres.more.map((facet) => (
                <option key={facet.value} value={facet.value}>
                  {facet.value} ({facet.count})
                </option>
              ))}
            </select>
          )}
        </FacetRow>
      )}
      {decades.length > 0 && (
        <FacetRow label="Decade">
          {decades.map((facet) => (
            <Chip
              active={focus.decade === facet.value}
              count={facet.count}
              key={facet.value}
              label={decadeLabel(facet.value)}
              onClick={() =>
                set({
                  decade: focus.decade === facet.value ? null : facet.value,
                })
              }
            />
          ))}
        </FacetRow>
      )}
      {qualities.length > 0 && (
        <FacetRow label="Quality">
          {qualities.map((quality) => (
            <Chip
              active={focus.quality === quality}
              count={countOf(facets?.qualities, quality)}
              key={quality}
              label={qualityLabels[quality]}
              onClick={() =>
                set({ quality: focus.quality === quality ? null : quality })
              }
            />
          ))}
        </FacetRow>
      )}
      {activeFocusCount(focus) > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-white/6 pt-2.5 text-[0.8125rem]">
          <p className="min-w-0 text-zinc-400" aria-live="polite">
            {total === undefined
              ? "…"
              : `${total} album${total === 1 ? "" : "s"}`}
            {summary && <span className="text-zinc-500"> · {summary}</span>}
          </p>
          <button
            className="inline-flex items-center gap-1.5 text-zinc-300 hover:text-white"
            onClick={() => onChange(emptyFocus)}
            title="Clears the filters; the sort order stays"
            type="button"
          >
            <X className="h-3.5 w-3.5" /> Clear focus
          </button>
        </div>
      )}
    </section>
  );
}
