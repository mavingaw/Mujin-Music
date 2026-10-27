"use client";

import {
  genreChipTitle,
  genreHref,
  genreOriginCaption,
  genreDisplayName,
} from "@parson/music-sdk";
import type { GenreRef } from "@parson/music-sdk/types";
import { Pencil } from "lucide-react";
import Link from "next/link";

/**
 * Several genres as linked chips, like Roon's "Hard Bop / Jazz / Modal
 * Music" row. Each chip opens the genre page; its tooltip names the path in
 * the genre tree and where the genre came from.
 */
export function GenreChips({
  genres,
  onEdit,
  showOrigin = false,
  className = "",
  label = "Genres",
}: {
  genres: readonly GenreRef[] | null | undefined;
  /** Shown to administrators: opens the genre editor. */
  onEdit?: () => void;
  showOrigin?: boolean;
  className?: string;
  label?: string;
}) {
  const list = genres ?? [];
  if (!list.length && !onEdit) return null;
  const origin = showOrigin ? genreOriginCaption(list) : null;
  return (
    <div className={`flex flex-wrap items-center gap-1.5 ${className}`}>
      <ul aria-label={label} className="contents">
        {list.map((genre) => (
          <li key={genre.slug}>
            <Link
              className="inline-flex items-center gap-1 rounded-full border border-white/12 bg-white/6 px-2.5 py-0.5 text-xs font-medium text-zinc-200 transition-colors hover:border-white/25 hover:bg-white/12 hover:text-white"
              href={genreHref(genre.slug)}
              title={genreChipTitle(genre)}
            >
              {genreDisplayName(genre.name)}
            </Link>
          </li>
        ))}
      </ul>
      {origin && <span className="text-[11px] text-zinc-500">{origin}</span>}
      {onEdit && (
        <button
          aria-label="Edit genres"
          className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs text-zinc-500 transition-colors hover:bg-white/8 hover:text-white"
          onClick={onEdit}
          type="button"
        >
          <Pencil className="h-3 w-3" />
          {list.length ? "Edit" : "Add genres"}
        </button>
      )}
    </div>
  );
}
