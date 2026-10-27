"use client";

import { factSourceLabel } from "@parson/music-sdk";
import type { ResolvedFact } from "@parson/music-sdk/types";
import { Globe2, Lock, Tag } from "lucide-react";

const STYLE = {
  edited: {
    icon: Lock,
    className:
      "border-mujin-copper/25 bg-mujin-copper/10 text-mujin-copper-hover",
    text: "Edited",
  },
  tags: {
    icon: Tag,
    className: "border-white/10 bg-white/[0.04] text-zinc-400",
    text: "Tags",
  },
  online: {
    icon: Globe2,
    className: "border-white/15 bg-white/[0.05] text-mujin-text-secondary",
    text: "MusicBrainz",
  },
} as const;

/**
 * Where a fact comes from: the file tags, MusicBrainz or a frozen edit. A
 * fallback (the preferred source had nothing) is marked with a dashed edge.
 */
export function FactSourceBadge({
  fact,
  compact = false,
}: {
  fact: ResolvedFact;
  compact?: boolean;
}) {
  if (!fact.source) return null;
  const style = STYLE[fact.source];
  const Icon = style.icon;
  const label = factSourceLabel(fact) ?? style.text;
  return (
    <span
      aria-label={`Source: ${label}`}
      className={`inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide ${style.className} ${
        fact.fallback ? "border-dashed" : ""
      }`}
      title={label}
    >
      <Icon aria-hidden className="h-2.5 w-2.5" />
      {!compact && style.text}
    </span>
  );
}
