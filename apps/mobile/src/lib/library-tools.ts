import { palette } from "@/constants/colors";
import type {
  Breakdown,
  FileQualityTier,
  SkippedFile,
  ToolJob,
} from "@parson/music-sdk";

/** Tier colours, matching the signal-path light and the web tools. */
export const TIER_COLORS: Record<FileQualityTier, string> = {
  dsd: palette.accentDeep,
  hi_res: palette.accent,
  lossless: palette.accentSoft,
  lossy: palette.qualityLow,
  unknown: palette.muted,
};

export const TIER_LABELS: Record<FileQualityTier, string> = {
  dsd: "DSD",
  hi_res: "Hi-Res",
  lossless: "CD quality",
  lossy: "Lossy",
  unknown: "Unknown",
};

export function tierColor(key: string): string {
  return key in TIER_COLORS
    ? TIER_COLORS[key as FileQualityTier]
    : TIER_COLORS.unknown;
}

export function tierLabel(key: string, fallback: string): string {
  return key in TIER_LABELS ? TIER_LABELS[key as FileQualityTier] : fallback;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatPlayingTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0 min";
  const minutes = Math.round(seconds / 60);
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  if (days > 0) return hours ? `${days} d ${hours} hr` : `${days} d`;
  if (hours > 0) return `${hours} hr ${minutes % 60} min`;
  return `${minutes} min`;
}

/** Share of tracks that are lossless (DSD, Hi-Res or CD quality), 0..100. */
export function losslessPercent(tiers: Breakdown[]): number {
  const total = tiers.reduce((sum, tier) => sum + tier.tracks, 0);
  if (!total) return 0;
  const lossless = tiers
    .filter((tier) => ["dsd", "hi_res", "lossless"].includes(tier.key))
    .reduce((sum, tier) => sum + tier.tracks, 0);
  return Math.round((lossless / total) * 100);
}

/** Skipped files not ignored, grouped by reason, largest group first. */
export function skippedByReason(
  items: SkippedFile[],
): { label: string; items: SkippedFile[] }[] {
  const groups = new Map<string, { label: string; items: SkippedFile[] }>();
  for (const item of items) {
    if (item.ignored) continue;
    const group = groups.get(item.reason) ?? { label: item.label, items: [] };
    group.items.push(item);
    groups.set(item.reason, group);
  }
  return [...groups.values()].sort((a, b) => b.items.length - a.items.length);
}

export function jobLine(job: ToolJob): string {
  if (job.state !== "running") return job.message || job.title;
  const progress =
    job.total > 0
      ? `${Math.min(job.done, job.total)} of ${job.total}`
      : job.done > 0
        ? `${job.done} checked`
        : "Starting…";
  return job.current ? `${progress} · ${job.current}` : progress;
}
