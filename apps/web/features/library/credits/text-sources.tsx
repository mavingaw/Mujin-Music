"use client";

import {
  editorialSourceName,
  editorialSourceShortName,
  reviewStars,
} from "@parson/music-sdk";
import type {
  ReviewScore,
  ReviewScores as ReviewScoresValue,
  TextSource,
} from "@parson/music-sdk/types";
import { Star, StarHalf } from "lucide-react";
import { useCallback, useState } from "react";

const STORAGE_PREFIX = "parson.text-source.";

function readStored(key: string) {
  try {
    return window.localStorage.getItem(STORAGE_PREFIX + key);
  } catch {
    return null;
  }
}

/**
 * The text source a listener picked for one album or artist, remembered
 * in this browser. Without a pick the page follows the administrator's
 * priority list.
 */
export function usePickedTextSource(key: string) {
  const [picked, setPicked] = useState<string | null>(() =>
    typeof window === "undefined" ? null : readStored(key),
  );
  const pick = useCallback(
    (id: string) => {
      setPicked(id);
      try {
        window.localStorage.setItem(STORAGE_PREFIX + key, id);
      } catch {
        // Private windows may refuse storage; the pick still applies here.
      }
    },
    [key],
  );
  return [picked, pick] as const;
}

/**
 * Roon's source switcher under a review: one pill per editorial source
 * that has text, the active one filled.
 */
export function TextSourceSwitcher({
  sources,
  active,
  onPick,
  has,
}: {
  sources: readonly TextSource[];
  active: string | null | undefined;
  onPick: (id: string) => void;
  /** Which sources to offer (those with the text the page shows). */
  has?: (source: TextSource) => boolean;
}) {
  const offered = has ? sources.filter(has) : sources;
  if (offered.length < 2) return null;
  return (
    <div
      aria-label="Text source"
      className="flex flex-wrap items-center gap-1.5"
      role="radiogroup"
    >
      <span className="mr-1 text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">
        Source
      </span>
      {offered.map((source) => {
        const selected = source.id === active;
        return (
          <button
            aria-checked={selected}
            className={`rounded-full border px-2.5 py-1 text-xs font-medium transition-colors ${
              selected
                ? "border-white/40 bg-white/12 text-white"
                : "border-white/10 text-zinc-400 hover:border-white/25 hover:text-zinc-100"
            }`}
            key={source.id}
            onClick={() => onPick(source.id)}
            role="radio"
            title={editorialSourceName(source.id)}
            type="button"
          >
            {editorialSourceShortName(source.id)}
          </button>
        );
      })}
    </div>
  );
}

function Stars({ score }: { score: ReviewScore }) {
  const stars = reviewStars(score);
  if (!stars) {
    return (
      <span className="font-semibold tabular-nums text-zinc-100">
        {score.rating}
      </span>
    );
  }
  return (
    <span
      aria-label={`${score.rating} (${stars.value} of 5 stars)`}
      className="flex items-center gap-0.5 text-amber-300"
      role="img"
      title={score.rating}
    >
      {Array.from({ length: stars.full }, (_, index) => (
        <Star
          aria-hidden
          className="h-3.5 w-3.5 fill-current"
          key={`full-${index}`}
        />
      ))}
      {stars.half > 0 && (
        <span aria-hidden className="relative h-3.5 w-3.5">
          <Star className="absolute inset-0 h-3.5 w-3.5 text-white/15" />
          <StarHalf className="absolute inset-0 h-3.5 w-3.5 fill-current" />
        </span>
      )}
      {Array.from({ length: stars.empty }, (_, index) => (
        <Star
          aria-hidden
          className="h-3.5 w-3.5 text-white/15"
          key={`empty-${index}`}
        />
      ))}
    </span>
  );
}

/** The publications that reviewed an album, with their ratings. */
export function ReviewScores({ scores }: { scores: ReviewScoresValue }) {
  return (
    <section
      aria-label="Professional ratings"
      className="mt-7 rounded-xl border border-white/8 bg-white/[0.025] p-4"
    >
      <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-zinc-400">
        Professional ratings
      </h3>
      <ul className="grid gap-x-8 gap-y-2 sm:grid-cols-2">
        {scores.scores.map((score) => (
          <li
            className="flex min-w-0 items-center justify-between gap-3 border-b border-white/5 pb-2 text-sm last:border-0 sm:[&:nth-last-child(2):nth-child(odd)]:border-0"
            key={score.publication}
          >
            <span className="min-w-0 truncate text-zinc-300">
              {score.publication}
            </span>
            <Stars score={score} />
          </li>
        ))}
      </ul>
      <p className="mt-3 text-xs leading-5 text-zinc-500">
        As listed in the {editorialSourceName(`wikipedia:${scores.language}`)}{" "}
        article{" "}
        <a
          className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
          href={scores.url}
          rel="noreferrer"
          target="_blank"
        >
          “{scores.title}”
        </a>
        , licensed under{" "}
        <a
          className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
          href={scores.license_url}
          rel="noreferrer license"
          target="_blank"
        >
          {scores.license}
        </a>
        .
      </p>
    </section>
  );
}
