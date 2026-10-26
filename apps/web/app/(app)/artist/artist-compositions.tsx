"use client";

import {
  catalogueHeadline,
  catalogueStatusText,
  composedLine,
  composerWorkFacets,
  filterComposerWorks,
  groupWorksByForm,
  libraryLine,
  notableWorks,
  workHref,
  type CatalogueScope,
  type ComposerWork,
  type ComposerWorks,
  type WorkFacetValue,
  type WorkFilter,
} from "@parson/music-sdk";
import { Library, Loader2, Sparkles } from "lucide-react";
import Link from "next/link";
import { useMemo, useState } from "react";

/** Rows shown per form before "Show all". */
const GROUP_PREVIEW = 8;

function ChipRow({
  label,
  values,
  selected,
  onSelect,
}: {
  label: string;
  values: WorkFacetValue[];
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

function workDetails(entry: ComposerWork) {
  const title = entry.work.title.toLowerCase();
  return [
    entry.work.key && !title.includes(entry.work.key.toLowerCase())
      ? entry.work.key
      : null,
    entry.work.catalogue && !title.includes(entry.work.catalogue.toLowerCase())
      ? entry.work.catalogue
      : null,
    entry.work.period,
  ].filter(Boolean) as string[];
}

/** Whether the work is in the library, as a small badge. */
function LibraryBadge({ entry }: { entry: ComposerWork }) {
  if (entry.track_count > 0) {
    return (
      <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-emerald-400/12 px-2 py-0.5 text-[11px] font-semibold text-emerald-300">
        <Library aria-hidden className="h-3 w-3" />
        {entry.recording_count || 1}{" "}
        {(entry.recording_count || 1) === 1 ? "recording" : "recordings"}
      </span>
    );
  }
  return (
    <span className="shrink-0 text-[11px] font-medium text-zinc-600">
      Not in your library
    </span>
  );
}

/** A notable work as a card: title, form, "Composed …" and library state. */
export function NotableWorkCard({ entry }: { entry: ComposerWork }) {
  const composed = composedLine(entry.work);
  const facts = [entry.work.form, ...workDetails(entry)].filter(Boolean);
  return (
    <Link
      className={`group relative flex min-h-[7.5rem] flex-col justify-between overflow-hidden rounded-xl border p-4 transition-colors ${
        entry.track_count > 0
          ? "border-white/12 bg-gradient-to-br from-white/[0.07] to-white/[0.02] hover:border-white/25"
          : "border-white/8 bg-white/[0.02] hover:border-white/18"
      }`}
      href={workHref(entry.work.mbid)}
    >
      <div className="min-w-0">
        {entry.notable_rank != null && (
          <p className="mb-1.5 flex items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.16em] text-amber-200/80">
            <Sparkles aria-hidden className="h-3 w-3" />
            Notable
          </p>
        )}
        <p className="line-clamp-2 text-[15px] font-semibold leading-snug text-white group-hover:underline group-hover:underline-offset-2">
          {entry.work.title}
        </p>
        {facts.length > 0 && (
          <p className="mt-1 truncate text-xs text-zinc-400">
            {facts.join(" · ")}
          </p>
        )}
      </div>
      <div className="mt-3 flex items-center justify-between gap-3">
        <span className="text-xs text-zinc-500">{composed ?? ""}</span>
        <LibraryBadge entry={entry} />
      </div>
    </Link>
  );
}

const SCOPES: { key: CatalogueScope; label: string }[] = [
  { key: "all", label: "All works" },
  { key: "library", label: "In your library" },
  { key: "notable", label: "Notable" },
];

/**
 * A composer's compositions: the whole MusicBrainz catalogue with what is
 * in the library marked, the notable works Wikidata names first, then
 * every piece grouped by form (Symphonies, Concertos, Sonatas, ...) with
 * filter chips for form, period and key.
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
  const scope: CatalogueScope = filter.scope ?? "all";
  const inScope = useMemo(
    () => filterComposerWorks(works.works, { scope }),
    [scope, works.works],
  );
  const facets = useMemo(() => composerWorkFacets(inScope), [inScope]);
  const groups = useMemo(
    () => groupWorksByForm(filterComposerWorks(works.works, filter)),
    [filter, works.works],
  );
  const notable = useMemo(() => notableWorks(works.works, 8), [works.works]);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const filtered = Boolean(filter.form || filter.period || filter.key);
  const catalogue = works.catalogue;
  const status = catalogueStatusText(catalogue);
  const hasCatalogue = works.works.some((entry) => entry.in_catalogue);

  if (!works.works.length) {
    return (
      <div className="space-y-2 text-sm leading-6 text-zinc-500">
        <p>
          No compositions yet. Works appear here when online metadata links your
          tracks to MusicBrainz works.
        </p>
        {status && <p>{status}</p>}
      </div>
    );
  }

  return (
    <div className="space-y-10">
      <section
        aria-label="Catalogue"
        className="flex flex-col gap-4 rounded-2xl border border-white/8 bg-gradient-to-br from-white/[0.05] via-white/[0.02] to-transparent p-5 sm:flex-row sm:items-end sm:justify-between"
      >
        <div className="min-w-0">
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-zinc-500">
            {hasCatalogue ? "Complete catalogue" : "In your library"}
          </p>
          <p className="mt-1 text-lg font-semibold text-white">
            {catalogueHeadline(works.works, catalogue)}
          </p>
          {hasCatalogue && (
            <p className="mt-1 text-xs leading-5 text-zinc-500">
              Every work from{" "}
              {catalogue?.musicbrainz_url ? (
                <a
                  className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
                  href={catalogue.musicbrainz_url}
                  rel="noreferrer"
                  target="_blank"
                >
                  MusicBrainz
                </a>
              ) : (
                "MusicBrainz"
              )}
              {catalogue?.notable ? (
                <>
                  ; notable works, years, keys and forms from{" "}
                  {catalogue.wikidata_url ? (
                    <a
                      className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
                      href={catalogue.wikidata_url}
                      rel="noreferrer"
                      target="_blank"
                    >
                      Wikidata
                    </a>
                  ) : (
                    "Wikidata"
                  )}
                </>
              ) : null}
              . Both are CC0.
            </p>
          )}
          {status && (
            <p
              className="mt-2 flex items-center gap-1.5 text-xs leading-5 text-zinc-400"
              role="status"
            >
              {catalogue?.state === "pending" && (
                <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
              )}
              {status}
            </p>
          )}
        </div>
        {hasCatalogue && (
          <div
            aria-label="Show"
            className="flex shrink-0 rounded-full bg-black/30 p-1"
            role="radiogroup"
          >
            {SCOPES.map((option) => (
              <button
                aria-checked={scope === option.key}
                className={`rounded-full px-3 py-1.5 text-xs font-semibold transition-colors ${
                  scope === option.key
                    ? "bg-white text-black"
                    : "text-zinc-400 hover:text-white"
                }`}
                key={option.key}
                onClick={() =>
                  onFilterChange({
                    scope: option.key === "all" ? null : option.key,
                  })
                }
                role="radio"
                type="button"
              >
                {option.label}
              </button>
            ))}
          </div>
        )}
      </section>

      {scope === "all" && !filtered && notable.length > 0 && (
        <section aria-label="Notable compositions">
          <h3 className="mb-4 text-xl font-semibold text-white">
            Notable compositions
          </h3>
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            {notable.map((entry) => (
              <NotableWorkCard entry={entry} key={entry.work.mbid} />
            ))}
          </div>
        </section>
      )}

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
            onClick={() => onFilterChange({ scope: filter.scope })}
            type="button"
          >
            Clear filters
          </button>
        )}
      </div>

      {groups.length === 0 && (
        <p className="text-sm text-zinc-500">No compositions match.</p>
      )}

      {groups.map((group) => {
        const open = expanded.has(group.label) || filtered;
        const shown = open ? group.works : group.works.slice(0, GROUP_PREVIEW);
        const owned = group.works.filter(
          (entry) => entry.track_count > 0,
        ).length;
        return (
          <section key={group.label}>
            <h3 className="mb-3 flex flex-wrap items-baseline gap-x-3 text-xl font-semibold text-white">
              {group.label}
              <span className="text-sm font-medium text-zinc-500">
                {group.works.length.toLocaleString("en-US")}
                {hasCatalogue && owned > 0 && ` · ${owned} in your library`}
              </span>
            </h3>
            <ul className="divide-y divide-white/6">
              {shown.map((entry) => {
                const details = workDetails(entry);
                const composed = composedLine(entry.work);
                return (
                  <li key={entry.work.mbid}>
                    <Link
                      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 rounded-md px-2 py-3 transition-colors hover:bg-white/4"
                      href={workHref(entry.work.mbid)}
                    >
                      <div className="min-w-0">
                        <p
                          className={`truncate text-sm font-medium ${
                            entry.track_count > 0
                              ? "text-white"
                              : "text-zinc-300"
                          }`}
                        >
                          {entry.notable_rank != null && (
                            <Sparkles
                              aria-label="Notable"
                              className="mr-1.5 inline h-3.5 w-3.5 -translate-y-px text-amber-200/80"
                            />
                          )}
                          {entry.work.title}
                        </p>
                        {(composed || details.length > 0) && (
                          <p className="truncate text-xs text-zinc-500">
                            {[composed, ...details].filter(Boolean).join(" · ")}
                          </p>
                        )}
                      </div>
                      {hasCatalogue ? (
                        <LibraryBadge entry={entry} />
                      ) : (
                        <span className="text-right text-xs font-medium text-zinc-500">
                          {libraryLine(entry).replace("In your library · ", "")}
                        </span>
                      )}
                    </Link>
                  </li>
                );
              })}
            </ul>
            {group.works.length > shown.length && (
              <button
                className="mt-2 px-2 text-xs font-semibold text-zinc-400 transition-colors hover:text-white"
                onClick={() =>
                  setExpanded((current) => new Set(current).add(group.label))
                }
                type="button"
              >
                Show all {group.works.length.toLocaleString("en-US")}
              </button>
            )}
          </section>
        );
      })}
    </div>
  );
}
