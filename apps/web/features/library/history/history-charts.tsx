"use client";

import { cn } from "@/lib/cn";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import { defaultCover } from "@/lib/images/default-cover";
import { PersonAvatar } from "@/features/library/credits/person-avatar";
import { formatListeningTime, type HistoryRankedItem } from "@parson/music-sdk";
import Image from "next/image";
import Link from "next/link";
import { useState } from "react";

export interface BarDatum {
  key: string;
  label: string;
  title: string;
  value: number;
  detail?: string;
}

/**
 * A single-series column chart: thin columns with 2px gaps, rounded data
 * ends on the baseline, a recessive axis and a hover readout. One series,
 * so no legend; the section title names it.
 */
export function ColumnChart({
  data,
  height = 160,
  labelEvery = 1,
  ariaLabel,
}: {
  data: BarDatum[];
  height?: number;
  labelEvery?: number;
  ariaLabel: string;
}) {
  const [active, setActive] = useState<number | null>(null);
  const max = Math.max(1, ...data.map((datum) => datum.value));
  const shown = active === null ? null : data[active];
  return (
    <figure aria-label={ariaLabel} className="w-full">
      <div className="mb-2 h-5 text-xs text-zinc-400" aria-live="polite">
        {shown ? (
          <>
            <span className="font-semibold text-zinc-100">{shown.title}</span>
            {" · "}
            {shown.value} {shown.value === 1 ? "play" : "plays"}
            {shown.detail ? ` · ${shown.detail}` : ""}
          </>
        ) : (
          <span className="text-zinc-600">Hover a column for details</span>
        )}
      </div>
      <div
        className="relative flex items-end gap-[2px] border-b border-white/10"
        onMouseLeave={() => setActive(null)}
        style={{ height }}
      >
        {[0.5, 1].map((line) => (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 border-t border-dashed border-white/[0.06]"
            key={line}
            style={{ bottom: `${line * 100}%` }}
          />
        ))}
        {data.map((datum, index) => (
          <button
            aria-label={`${datum.title}: ${datum.value} plays`}
            className="group relative flex h-full min-w-0 flex-1 items-end focus-visible:outline-none"
            key={datum.key}
            onBlur={() => setActive(null)}
            onFocus={() => setActive(index)}
            onMouseEnter={() => setActive(index)}
            type="button"
          >
            <span
              className={cn(
                "block w-full rounded-t-[4px] transition-colors",
                datum.value > 0 ? "bg-mujin-copper/80" : "bg-white/[0.06]",
                active === index && "bg-mujin-copper",
              )}
              style={{
                height:
                  datum.value > 0
                    ? `${Math.max(3, (datum.value / max) * 100)}%`
                    : "2px",
              }}
            />
          </button>
        ))}
      </div>
      <div className="mt-1.5 flex gap-[2px]" aria-hidden="true">
        {data.map((datum, index) => (
          <span
            className="min-w-0 flex-1 truncate text-center text-[0.625rem] tabular-nums text-zinc-600"
            key={datum.key}
          >
            {index % labelEvery === 0 ? datum.label : ""}
          </span>
        ))}
      </div>
    </figure>
  );
}

export function StatTile({
  label,
  value,
  change,
  hint,
}: {
  label: string;
  value: string;
  change?: string | null;
  hint?: string;
}) {
  const negative = change?.startsWith("−");
  return (
    <div className="rounded-2xl border border-white/8 bg-white/[0.025] px-4 py-3.5">
      <p className="text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-zinc-500">
        {label}
      </p>
      <p className="mt-1 text-2xl font-bold tabular-nums text-white">{value}</p>
      <p className="mt-0.5 h-4 truncate text-xs text-zinc-500">
        {change && (
          <span
            className={cn(
              "mr-1 font-semibold tabular-nums",
              negative ? "text-zinc-400" : "text-emerald-300",
            )}
          >
            {change}
          </span>
        )}
        {change ? "vs previous" : hint}
      </p>
    </div>
  );
}

function rankHref(kind: "artist" | "album" | "track", item: HistoryRankedItem) {
  if (!item.available) return null;
  if (kind === "artist") return `/artist?id=${encodeURIComponent(item.id)}`;
  if (kind === "album") return `/album?id=${encodeURIComponent(item.id)}`;
  return item.detailId
    ? `/artist?id=${encodeURIComponent(item.detailId)}`
    : null;
}

/** A ranked list with a magnitude bar per row (single hue, one scale). */
export function RankedList({
  items,
  kind,
  onPlay,
}: {
  items: HistoryRankedItem[];
  kind: "artist" | "album" | "track";
  onPlay?: (item: HistoryRankedItem) => void;
}) {
  const max = Math.max(1, ...items.map((item) => item.plays));
  if (!items.length) {
    return (
      <p className="py-6 text-sm text-zinc-500">No plays in this period.</p>
    );
  }
  return (
    <ol className="space-y-1">
      {items.map((item, index) => {
        const image = item.imagePath
          ? getLibraryImageUrl(item.imagePath, getBaseURL)
          : null;
        const href = rankHref(kind, item);
        const body = (
          <>
            <span className="w-5 shrink-0 text-right text-xs font-semibold tabular-nums text-zinc-500">
              {index + 1}
            </span>
            {kind === "artist" ? (
              <PersonAvatar imageUrl={image} name={item.name} size={40} />
            ) : (
              <Image
                alt=""
                className="h-10 w-10 shrink-0 rounded-md bg-zinc-900 object-cover"
                height={80}
                src={image ?? defaultCover}
                width={80}
              />
            )}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium text-zinc-100">
                {item.name}
              </span>
              <span className="mt-1 flex items-center gap-2">
                <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/[0.05]">
                  <span
                    className="block h-full rounded-full bg-mujin-copper/80"
                    style={{ width: `${(item.plays / max) * 100}%` }}
                  />
                </span>
                <span className="w-24 shrink-0 truncate text-right text-[0.6875rem] tabular-nums text-zinc-500">
                  {item.plays} {item.plays === 1 ? "play" : "plays"}
                </span>
              </span>
              {item.detail && (
                <span className="block truncate text-xs text-zinc-500">
                  {item.detail}
                </span>
              )}
            </span>
          </>
        );
        return (
          <li key={item.id}>
            {kind === "track" && onPlay && item.available ? (
              <button
                className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-white/[0.04]"
                onClick={() => onPlay(item)}
                title={`Play · ${formatListeningTime(item.listenedSeconds)} listened`}
                type="button"
              >
                {body}
              </button>
            ) : href ? (
              <Link
                className="flex items-center gap-3 rounded-lg px-2 py-1.5 transition-colors hover:bg-white/[0.04]"
                href={href}
                title={`${formatListeningTime(item.listenedSeconds)} listened`}
              >
                {body}
              </Link>
            ) : (
              <div className="flex items-center gap-3 px-2 py-1.5 opacity-70">
                {body}
              </div>
            )}
          </li>
        );
      })}
    </ol>
  );
}
