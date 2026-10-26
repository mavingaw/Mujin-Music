"use client";

import type { AudioFormatSummary, AudioQualityTier } from "@parson/music-sdk";
import { cn } from "@/lib/cn";
import { audioBadgeDescription, audioBadgeLabel } from "./audio-quality";

export const AUDIO_TIER_STYLES: Record<AudioQualityTier, string> = {
  dsd: "border-fuchsia-300/30 bg-fuchsia-400/10 text-fuchsia-100",
  hi_res: "border-amber-200/30 bg-amber-300/10 text-amber-100",
  lossless: "border-sky-200/25 bg-sky-300/10 text-sky-100",
  lossy: "border-white/10 bg-white/5 text-zinc-300",
  unknown: "border-white/10 bg-white/5 text-zinc-300",
};

const TIER_MARK: Partial<Record<AudioQualityTier, string>> = {
  dsd: "DSD",
  hi_res: "Hi-Res",
};

/** A compact format chip: `Hi-Res FLAC 24/96`, `DSD64`, `MP3 320k`. */
export function AudioQualityBadge({
  className,
  summary,
  suffix,
}: {
  className?: string;
  summary: AudioFormatSummary;
  suffix?: string;
}) {
  const mark = summary.tier === "dsd" ? null : TIER_MARK[summary.tier];
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-1 rounded-full border px-2 text-[10.5px] font-semibold uppercase leading-none tracking-[0.06em] tabular-nums",
        AUDIO_TIER_STYLES[summary.tier],
        className,
      )}
      title={audioBadgeDescription(summary)}
    >
      {mark ? <span className="opacity-70">{mark}</span> : null}
      <span>{audioBadgeLabel(summary)}</span>
      {summary.channels && !["stereo", "mono"].includes(summary.channels) ? (
        <span className="opacity-70">{summary.channels}</span>
      ) : null}
      {suffix ? <span className="opacity-60">{suffix}</span> : null}
    </span>
  );
}
