import type {
  ImportItem,
  ImportItemAction,
  ImportJob,
  ImportJobSummary,
  ImportServiceId,
  ProviderCapability,
} from "@parson/music-sdk";

export type ReviewFilter =
  "review" | "unmatched" | "matched" | "skipped" | "all";

export const REVIEW_FILTERS: { id: ReviewFilter; label: string }[] = [
  { id: "review", label: "Needs review" },
  { id: "unmatched", label: "Not found" },
  { id: "matched", label: "Matched" },
  { id: "skipped", label: "Skipped" },
  { id: "all", label: "All" },
];

export function itemMatchesFilter(item: ImportItem, filter: ReviewFilter) {
  switch (filter) {
    case "all":
      return true;
    case "matched":
      return item.status === "matched" || item.status === "confirmed";
    default:
      return item.status === filter;
  }
}

export function filterCounts(
  items: ImportItem[],
): Record<ReviewFilter, number> {
  const counts: Record<ReviewFilter, number> = {
    review: 0,
    unmatched: 0,
    matched: 0,
    skipped: 0,
    all: items.length,
  };
  for (const item of items) {
    for (const filter of [
      "review",
      "unmatched",
      "matched",
      "skipped",
    ] as const) {
      if (itemMatchesFilter(item, filter)) counts[filter] += 1;
    }
  }
  return counts;
}

/** The first filter worth showing: whatever still needs a decision. */
export function defaultFilter(items: ImportItem[]): ReviewFilter {
  const counts = filterCounts(items);
  if (counts.review > 0) return "review";
  if (counts.unmatched > 0) return "unmatched";
  return "all";
}

/** Accept every suggestion still waiting for review. */
export function acceptAllSuggestions(items: ImportItem[]): ImportItemAction[] {
  return items
    .filter((item) => item.status === "review" && item.song_id)
    .map((item) => ({ action: "accept", position: item.position }));
}

/** Leave every track that has no match out of the playlist. */
export function skipAllUnmatched(items: ImportItem[]): ImportItemAction[] {
  return items
    .filter((item) => item.status === "unmatched")
    .map((item) => ({ action: "skip", position: item.position }));
}

export function includedSongIds(job: Pick<ImportJob, "items">): string[] {
  const seen = new Set<string>();
  for (const item of job.items) {
    if (
      (item.status === "matched" || item.status === "confirmed") &&
      item.song_id
    ) {
      seen.add(item.song_id);
    }
  }
  return [...seen];
}

export function formatTrackDuration(milliseconds: number | null | undefined) {
  if (!milliseconds || milliseconds <= 0) return "";
  const total = Math.round(milliseconds / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function percent(value: number) {
  return `${Math.round(Math.max(0, Math.min(1, value)) * 100)}%`;
}

export function jobProgressLabel(job: ImportJobSummary): string {
  switch (job.status) {
    case "fetching":
      return job.fetched
        ? `Reading tracks… ${job.fetched.toLocaleString()}`
        : "Reading tracks…";
    case "matching":
      return "Matching to your library…";
    case "failed":
      return job.error ?? "Import failed.";
    case "ready": {
      const considered = job.total - job.skipped;
      const parts = [`${job.matched} of ${considered} matched`];
      if (job.review) parts.push(`${job.review} to review`);
      if (job.unmatched) parts.push(`${job.unmatched} not found`);
      return parts.join(" · ");
    }
  }
}

export const SERVICE_ACCENTS: Record<ImportServiceId, string> = {
  spotify: "bg-[#1ed760]",
  apple_music: "bg-[#fa2d48]",
  youtube_music: "bg-[#ff0033]",
  deezer: "bg-[#a238ff]",
  file: "bg-zinc-400",
};

export const SERVICE_SHORT_NAMES: Record<ImportServiceId, string> = {
  spotify: "Spotify",
  apple_music: "Apple Music",
  youtube_music: "YouTube Music",
  deezer: "Deezer",
  file: "File",
};

export const CAPABILITY_ORDER: ProviderCapability[] = [
  "library-import",
  "metadata",
  "handoff",
  "playback",
];

/** Spotify redirects back to the imports page on this server. */
export function spotifyRedirectUri(origin: string) {
  return `${origin.replace(/\/$/, "")}/imports`;
}

/** Spotify only accepts https or http://127.0.0.1 redirect URIs. */
export function spotifyRedirectAllowed(origin: string) {
  return (
    origin.startsWith("https://") ||
    /^http:\/\/(127\.0\.0\.1|\[::1\])(:\d+)?$/.test(origin.replace(/\/$/, ""))
  );
}
