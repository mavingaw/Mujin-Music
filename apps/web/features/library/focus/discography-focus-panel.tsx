"use client";

import {
  focusResultLabel,
  focusSelectionCount,
  removeFocusChip,
  splitFocusGroups,
  toggleFocusValue,
  type FocusChip,
  type FocusGroup,
  type FocusSelection,
  type FocusSummary,
} from "@parson/music-sdk";
import { Check, ChevronDown, Filter, Loader2, Search, X } from "lucide-react";
import { useMemo, useState } from "react";
import { cn } from "@/lib/cn";

const VISIBLE_VALUES = 6;

type Noun = { one: string; other: string };

function ChipCount({ chip }: { chip: FocusChip }) {
  return (
    <span
      className={cn(
        "ml-auto pl-3 text-xs tabular-nums",
        chip.selected ? "text-zinc-300" : "text-zinc-500",
      )}
    >
      {chip.count}
    </span>
  );
}

function GroupColumn({
  group,
  onToggle,
}: {
  group: FocusGroup;
  onToggle: (chip: FocusChip) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [filter, setFilter] = useState("");
  const needle = filter.trim().toLowerCase();
  const values = needle
    ? group.values.filter((chip) => chip.label.toLowerCase().includes(needle))
    : group.values;
  const shown = expanded ? values : values.slice(0, VISIBLE_VALUES);
  const hidden = values.length - shown.length;
  return (
    <section aria-label={group.label} className="min-w-0">
      <h3 className="mb-2 text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-zinc-500">
        {group.label}
      </h3>
      {group.values.length > 10 && (
        <label className="mb-2 flex items-center gap-2 rounded-md border border-white/8 bg-black/30 px-2 py-1 text-xs text-zinc-400 focus-within:border-white/25">
          <Search className="h-3.5 w-3.5 shrink-0" />
          <input
            aria-label={`Filter ${group.label.toLowerCase()}`}
            className="min-w-0 flex-1 bg-transparent text-zinc-200 outline-none placeholder:text-zinc-600"
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter"
            value={filter}
          />
        </label>
      )}
      <ul className="space-y-0.5">
        {shown.map((chip) => {
          const empty = chip.count === 0 && !chip.selected;
          return (
            <li key={chip.key}>
              <button
                aria-pressed={chip.selected}
                className={cn(
                  "flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-[0.8125rem] transition-colors",
                  chip.selected
                    ? "bg-white/12 text-white"
                    : "text-zinc-300 hover:bg-white/6 hover:text-white",
                  empty && "pointer-events-none opacity-40",
                )}
                disabled={empty}
                onClick={() => onToggle(chip)}
                type="button"
              >
                <span
                  aria-hidden
                  className={cn(
                    "flex h-4 w-4 shrink-0 items-center justify-center rounded-[5px] border",
                    chip.selected
                      ? "border-white bg-white text-black"
                      : "border-white/25",
                  )}
                >
                  {chip.selected && (
                    <Check className="h-3 w-3" strokeWidth={3} />
                  )}
                </span>
                <span className="min-w-0 truncate">{chip.label}</span>
                <ChipCount chip={chip} />
              </button>
            </li>
          );
        })}
      </ul>
      {hidden > 0 && (
        <button
          className="mt-1 px-2 text-xs font-medium text-zinc-400 hover:text-white"
          onClick={() => setExpanded(true)}
          type="button"
        >
          {hidden} more
        </button>
      )}
      {!values.length && (
        <p className="px-2 py-1 text-xs text-zinc-600">No match</p>
      )}
    </section>
  );
}

/**
 * Roon-style Focus: stackable filters over a discography or a work's
 * recordings. Values in one group widen the result, groups narrow it, and
 * every count says how many albums choosing that value would show.
 */
export function FocusPanel({
  summary,
  selection,
  onChange,
  loading = false,
  noun = { one: "album", other: "albums" },
  className,
  note,
}: {
  summary: FocusSummary | null | undefined;
  selection: FocusSelection;
  onChange: (selection: FocusSelection) => void;
  loading?: boolean;
  noun?: Noun;
  className?: string;
  /** A line under the groups, e.g. what a source facet covers. */
  note?: React.ReactNode;
}) {
  const [open, setOpen] = useState(() => focusSelectionCount(selection) > 0);
  const [showMore, setShowMore] = useState(false);
  const groups = useMemo(
    () => splitFocusGroups(summary?.groups ?? []),
    [summary?.groups],
  );
  if (!summary || summary.total === 0) return null;
  const quality = summary.groups.find((group) => group.key === "quality");
  const hires = quality?.values.find((chip) => chip.key === "hires");
  const active = summary.selected.length;
  const filterable = summary.groups.length > 0;
  const toggle = (group: FocusGroup["key"], chip: FocusChip) =>
    onChange(toggleFocusValue(selection, group, chip.key));
  const columns = showMore
    ? [...groups.primary, ...groups.more]
    : groups.primary;

  return (
    <div
      className={cn(
        "rounded-2xl border border-white/8 bg-linear-to-b from-white/5 to-white/2 shadow-[0_18px_50px_-30px_rgba(0,0,0,0.9)]",
        className,
      )}
    >
      <div className="flex flex-wrap items-center gap-2 px-3 py-2.5 sm:px-4">
        <button
          aria-controls="focus-groups"
          aria-expanded={open}
          className={cn(
            "inline-flex h-8 items-center gap-2 rounded-full px-3.5 text-[0.8125rem] font-semibold transition-colors",
            open || active
              ? "bg-white text-black"
              : "bg-white/8 text-zinc-100 hover:bg-white/14",
            !filterable && "pointer-events-none opacity-50",
          )}
          disabled={!filterable}
          onClick={() => setOpen((value) => !value)}
          type="button"
        >
          <Filter className="h-3.5 w-3.5" />
          Focus
          {active > 0 && (
            <span className="rounded-full bg-black px-1.5 text-[0.6875rem] leading-4 text-white tabular-nums">
              {active}
            </span>
          )}
          <ChevronDown
            className={cn(
              "h-3.5 w-3.5 transition-transform",
              open && "rotate-180",
            )}
          />
        </button>
        {hires && (
          <button
            aria-pressed={hires.selected}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-[0.75rem] font-bold uppercase tracking-[0.08em] transition-colors",
              hires.selected
                ? "border-amber-300/70 bg-amber-300 text-black"
                : "border-amber-300/35 text-amber-200 hover:border-amber-300/70 hover:bg-amber-300/10",
            )}
            onClick={() => toggle("quality", hires)}
            title="Lossless at more than CD resolution: 24-bit, or above 48 kHz"
            type="button"
          >
            Hi-Res only
            <span
              className={cn(
                "text-[0.6875rem] font-semibold tabular-nums",
                hires.selected ? "text-black/70" : "text-amber-200/70",
              )}
            >
              {hires.count}
            </span>
          </button>
        )}
        {summary.selected
          .filter((chip) => chip.key !== "quality=hires")
          .map((chip) => (
            <button
              aria-label={`Remove ${chip.label}`}
              className="group inline-flex h-8 max-w-60 items-center gap-1.5 rounded-full bg-white/12 pl-3 pr-2 text-[0.8125rem] font-medium text-white transition-colors hover:bg-white/18"
              key={chip.key}
              onClick={() => onChange(removeFocusChip(selection, chip.key))}
              type="button"
            >
              <span className="truncate">{chip.label}</span>
              <X className="h-3.5 w-3.5 shrink-0 text-zinc-400 group-hover:text-white" />
            </button>
          ))}
        <div className="ml-auto flex items-center gap-3 pl-2 text-[0.8125rem]">
          {loading && (
            <Loader2
              aria-label="Updating"
              className="h-3.5 w-3.5 animate-spin text-zinc-500"
            />
          )}
          <span
            aria-live="polite"
            className="tabular-nums text-zinc-400"
            data-testid="focus-result"
          >
            {focusResultLabel(summary, noun)}
          </span>
          {active > 0 && (
            <button
              className="font-medium text-zinc-300 hover:text-white"
              onClick={() => onChange({})}
              type="button"
            >
              Clear
            </button>
          )}
        </div>
      </div>
      {open && filterable && (
        <div
          className="border-t border-white/8 px-3 pb-4 pt-4 sm:px-4"
          id="focus-groups"
        >
          <div className="grid grid-cols-[repeat(auto-fill,minmax(14.5rem,1fr))] gap-x-6 gap-y-5">
            {columns.map((group) => (
              <GroupColumn
                group={group}
                key={group.key}
                onToggle={(chip) => toggle(group.key, chip)}
              />
            ))}
          </div>
          <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-zinc-500">
            {groups.more.length > 0 && (
              <button
                className="font-semibold text-zinc-300 hover:text-white"
                onClick={() => setShowMore((value) => !value)}
                type="button"
              >
                {showMore
                  ? "Fewer filters"
                  : `More filters (${groups.more
                      .map((group) => group.label.toLowerCase())
                      .join(", ")})`}
              </button>
            )}
            {note}
          </div>
        </div>
      )}
    </div>
  );
}
