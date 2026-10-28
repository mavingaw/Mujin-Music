"use client";

import {
  getLyricsAvailability,
  lyricsSourceLabel,
  type LyricsAvailability,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

/**
 * Which of `songIds` have lyrics (from their files, or cached from LRCLIB).
 * One batched request per list; answers are remembered by the SDK.
 */
export function useLyricsAvailability(songIds: readonly string[]) {
  const ids = useMemo(
    () => [...new Set(songIds.filter(Boolean))].sort(),
    [songIds],
  );
  const query = useQuery({
    queryKey: ["lyrics-availability", ids],
    queryFn: () => getLyricsAvailability(ids),
    enabled: ids.length > 0,
    staleTime: 5 * 60_000,
    retry: false,
  });
  return query.data ?? EMPTY;
}

const EMPTY: Record<string, LyricsAvailability> = {};

/** The words a marker reads out and shows on hover. */
export function lyricsMarkLabel(availability: LyricsAvailability) {
  const source = lyricsSourceLabel(availability.source);
  return `${availability.synced ? "Timed lyrics" : "Lyrics"}${source ? ` · ${source}` : ""}`;
}

/**
 * The small "timed lyrics" marker on track rows and in the player bar: three
 * staggered lines, copper when the lyrics are timed, grey when untimed.
 */
export function TimedLyricsMark({
  availability,
  className = "",
  showLabel = false,
}: {
  availability?: LyricsAvailability | null;
  className?: string;
  /** Show "LRC" / "Lyrics" beside the glyph (the player bar does). */
  showLabel?: boolean;
}) {
  if (!availability) return null;
  const label = lyricsMarkLabel(availability);
  return (
    <span
      aria-label={label}
      className={`inline-flex shrink-0 items-center gap-1 rounded-[4px] border px-1 py-px align-middle text-[0.5938rem] font-semibold uppercase leading-3 tracking-[0.08em] ${
        availability.synced
          ? "border-mujin-copper/35 bg-mujin-copper/[0.08] text-mujin-copper"
          : "border-white/12 text-mujin-text-tertiary"
      } ${className}`}
      data-lyrics-mark={availability.synced ? "timed" : "plain"}
      role="img"
      title={label}
    >
      <svg
        aria-hidden="true"
        className="h-2.5 w-2.5"
        fill="none"
        viewBox="0 0 10 10"
      >
        <path
          d="M1 2h5M3 5h6M1 8h4"
          stroke="currentColor"
          strokeLinecap="round"
          strokeWidth="1.4"
        />
      </svg>
      {availability.synced ? (showLabel ? "Timed lyrics" : "LRC") : "Lyrics"}
    </span>
  );
}
