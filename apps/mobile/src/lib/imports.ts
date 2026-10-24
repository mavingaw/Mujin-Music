import type {
  ImportItem,
  ImportItemAction,
  ImportJobSummary,
  ImportServiceId,
} from "@parson/music-sdk";

export type MobileReviewFilter = "review" | "unmatched" | "all";

export const SERVICE_COLORS: Record<ImportServiceId, string> = {
  spotify: "#1ed760",
  apple_music: "#fa2d48",
  youtube_music: "#ff0033",
  deezer: "#a238ff",
  file: "#a1a1aa",
};

export const SERVICE_NAMES: Record<ImportServiceId, string> = {
  spotify: "Spotify",
  apple_music: "Apple Music",
  youtube_music: "YouTube Music",
  deezer: "Deezer",
  file: "Playlist file",
};

export function itemsForFilter(
  items: ImportItem[],
  filter: MobileReviewFilter,
): ImportItem[] {
  if (filter === "all") return items;
  return items.filter((item) => item.status === filter);
}

/** Start where a decision is still needed. */
export function initialMobileFilter(items: ImportItem[]): MobileReviewFilter {
  if (items.some((item) => item.status === "review")) return "review";
  if (items.some((item) => item.status === "unmatched")) return "unmatched";
  return "all";
}

export function acceptSuggestionActions(
  items: ImportItem[],
): ImportItemAction[] {
  return items
    .filter((item) => item.status === "review" && item.song_id)
    .map((item) => ({ action: "accept", position: item.position }));
}

export function importStatusLine(job: ImportJobSummary): string {
  if (job.status === "fetching") {
    return job.fetched ? `Reading tracks… ${job.fetched}` : "Reading tracks…";
  }
  if (job.status === "matching") return "Matching to your library…";
  if (job.status === "failed") return job.error ?? "Import failed.";
  const considered = Math.max(0, job.total - job.skipped);
  const details = [`${job.matched}/${considered} matched`];
  if (job.review) details.push(`${job.review} to check`);
  if (job.unmatched) details.push(`${job.unmatched} not found`);
  return details.join(" · ");
}

export function confidencePercent(confidence: number): string {
  return `${Math.round(Math.max(0, Math.min(1, confidence)) * 100)}%`;
}

export function isJobRunning(job: Pick<ImportJobSummary, "status">): boolean {
  return job.status === "fetching" || job.status === "matching";
}
