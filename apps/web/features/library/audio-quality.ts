import {
  describeAudioFormat,
  type AudioFormatSummary,
  type AudioQualityTier,
} from "@parson/music-sdk";
import type { AudioProperties } from "@parson/music-sdk/types";

const TIER_RANK: Record<AudioQualityTier, number> = {
  dsd: 4,
  hi_res: 3,
  lossless: 2,
  lossy: 1,
  unknown: 0,
};

export type AlbumAudioSummary = AudioFormatSummary & {
  /** The album mixes formats or resolutions; the best one is shown. */
  mixed: boolean;
};

/** The best format on an album, and whether its tracks differ. */
export function albumAudioSummary(
  songs: ReadonlyArray<{ audio?: AudioProperties | null }>,
): AlbumAudioSummary | null {
  let best: AudioFormatSummary | null = null;
  let bestScore = -1;
  const labels = new Set<string>();
  for (const song of songs) {
    const summary = describeAudioFormat(song.audio);
    if (!summary) continue;
    labels.add(`${summary.name} ${summary.badge}`);
    const score =
      TIER_RANK[summary.tier] * 1e9 +
      (song.audio?.bit_depth ?? 0) * 1e6 +
      (song.audio?.sample_rate ?? 0);
    if (score > bestScore) {
      best = summary;
      bestScore = score;
    }
  }
  return best ? { ...best, mixed: labels.size > 1 } : null;
}

/** Badge text such as `FLAC 24/96`, `DSD64` or `MP3 320k`. */
export function audioBadgeLabel(summary: AudioFormatSummary) {
  if (summary.tier === "dsd") return summary.badge;
  if (summary.badge === summary.name) return summary.name;
  return `${summary.name} ${summary.badge}`;
}

/** Long description for titles and screen readers. */
export function audioBadgeDescription(summary: AudioFormatSummary) {
  const tier = {
    dsd: "Direct Stream Digital",
    hi_res: "High-resolution lossless",
    lossless: "Lossless",
    lossy: "Lossy",
    unknown: "Audio",
  }[summary.tier];
  return [tier, summary.name, summary.resolution, summary.channels]
    .filter(Boolean)
    .join(" · ");
}
