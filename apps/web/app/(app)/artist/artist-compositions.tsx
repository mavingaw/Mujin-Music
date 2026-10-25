"use client";

import {
  composerWorkFacets,
  filterComposerWorks,
  groupWorksByForm,
  type ComposerWorks,
  type FacetValue,
  type WorkFilter,
} from "@parson/music-sdk";
import Link from "next/link";
import { useMemo } from "react";

function ChipRow({
  label,
  values,
  selected,
  onSelect,
}: {
  label: string;
  values: FacetValue[];
  selected: string | null | undefined;
  onSelect: (value: string | null) => void;
}) {
  if (values.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="w-14 shrink-0 text-xs font-semibold uppercase tracking-[0.08em] text-zinc-500">
        {label}
      </span>
      {values.map((facet) => {
        const active = selected === facet.value;
        return (
          <button
            aria-pressed={active}
            className={`rounded-full px-3 py-1 text-sm font-medium transition-colors ${
              active
                ? "bg-white text-black"
                : "bg-white/6 text-zinc-300 hover:bg-white/10 hover:text-white"
            }`}
            key={facet.value}
            onClick={() => onSelect(active ? null : facet.value)}
            type="button"
          >
            {facet.value}
            <span
              className={`ml-1.5 text-xs ${active ? "text-black/60" : "text-zinc-500"}`}
            >
              {facet.count}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/**
 * A composer's works in the library, grouped by form (Symphonies,
 * Concertos, Sonatas, ...) with filter chips for form, period and key.
 */
export function ArtistCompositionsPanel({
  works,
  filter,
  onFilterChange,
}: {
  works: ComposerWorks;
  filter: WorkFilter;
  onFilterChange: (filter: WorkFilter) => void;
}) {
  const facets = useMemo(() => composerWorkFacets(works.works), [works.works]);
  const groups = useMemo(
    () => groupWorksByForm(filterComposerWorks(works.works, filter)),
    [filter, works.works],
  );
  const filtered = Boolean(filter.form || filter.period || filter.key);

  if (!works.works.length) {
    return (
      <p className="text-sm leading-6 text-zinc-500">
        No compositions yet. Works appear here when online metadata links your
        tracks to MusicBrainz works.
      </p>
    );
  }

  return (
    <div className="space-y-10">
      <div aria-label="Filter compositions" className="space-y-2.5">
        <ChipRow
          label="Form"
          onSelect={(form) => onFilterChange({ ...filter, form })}
          selected={filter.form}
          values={facets.forms}
        />
        <ChipRow
          label="Period"
          onSelect={(period) => onFilterChange({ ...filter, period })}
          selected={filter.period}
          values={facets.periods}
        />
        <ChipRow
          label="Key"
          onSelect={(key) => onFilterChange({ ...filter, key })}
          selected={filter.key}
          values={facets.keys}
        />
        {filtered && (
          <button
            className="text-xs font-semibold text-zinc-400 hover:text-white"
            onClick={() => onFilterChange({})}
            type="button"
          >
            Clear filters
          </button>
        )}
      </div>

      {groups.length === 0 && (
        <p className="text-sm text-zinc-500">No compositions match.</p>
      )}

      {groups.map((group) => (
        <section key={group.label}>
          <h3 className="mb-3 text-xl font-semibold text-white">
            {group.label}
            <span className="ml-2 text-sm font-medium text-zinc-500">
              {group.works.length}
            </span>
          </h3>
          <ul className="divide-y divide-white/6">
            {group.works.map(({ work, recording_count }) => {
              const details = [
                work.key,
                work.catalogue,
                work.composed,
                work.period,
              ].filter(Boolean);
              return (
                <li key={work.mbid}>
                  <Link
                    className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 rounded-md px-2 py-3 transition-colors hover:bg-white/4"
                    href={`/work?id=${encodeURIComponent(work.mbid)}`}
                  >
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium text-white">
                        {work.title}
                      </p>
                      {details.length > 0 && (
                        <p className="truncate text-xs text-zinc-500">
                          {details.join(" · ")}
                        </p>
                      )}
                    </div>
                    <span className="text-right text-xs font-medium text-zinc-500">
                      {recording_count}{" "}
                      {recording_count === 1 ? "recording" : "recordings"}
                    </span>
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}
