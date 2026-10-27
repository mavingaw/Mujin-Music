import type {
  Breakdown,
  DuplicateGroup,
  FileQualityTier,
  SkippedFile,
  SkipReason,
  ToolJob,
  ToolJobKind,
} from "@parson/music-sdk";

export const toolViews = [
  "overview",
  "skipped",
  "duplicates",
  "versions",
  "albumizer",
  "maintenance",
] as const;
export type ToolView = (typeof toolViews)[number];

export const toolViewLabels: Record<ToolView, string> = {
  overview: "Overview",
  skipped: "Skipped files",
  duplicates: "Duplicates",
  versions: "Album versions",
  albumizer: "Albumizer",
  maintenance: "Maintenance",
};

export function parseToolView(value: string | null | undefined): ToolView {
  return toolViews.includes(value as ToolView)
    ? (value as ToolView)
    : "overview";
}

/**
 * Tier colours follow the signal-path light: purple for lossless (deeper
 * for DSD and Hi-Res), amber for lossy.
 */
export const tierColors: Record<FileQualityTier, string> = {
  dsd: "#7c4dff",
  hi_res: "#b26bff",
  lossless: "#d6b8ff",
  lossy: "#f5c542",
  unknown: "#52525b",
};

export const tierShortLabels: Record<FileQualityTier, string> = {
  dsd: "DSD",
  hi_res: "Hi-Res",
  lossless: "CD quality",
  lossy: "Lossy",
  unknown: "Unknown",
};

export function isQualityTier(value: string): value is FileQualityTier {
  return value in tierColors;
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
  const digits = value >= 100 || unit === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

/** `3 days 4 hr`, `5 hr 12 min`, `42 min`. */
export function formatPlayingTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0 min";
  const minutes = Math.round(seconds / 60);
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const rest = minutes % 60;
  if (days > 0) return hours ? `${days} d ${hours} hr` : `${days} d`;
  if (hours > 0) return rest ? `${hours} hr ${rest} min` : `${hours} hr`;
  return `${rest} min`;
}

export function formatTrackTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0:00";
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = String(total % 60).padStart(2, "0");
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${secs}`
    : `${minutes}:${secs}`;
}

export function formatCount(value: number): string {
  return new Intl.NumberFormat("en").format(value);
}

/** Share of `part` in `whole` as a whole-number percentage. */
export function percent(part: number, whole: number): number {
  if (whole <= 0) return 0;
  return Math.round((part / whole) * 100);
}

/** Bar segments with widths that add up to 100 and never vanish. */
export function barSegments(
  items: Breakdown[],
): (Breakdown & { width: number })[] {
  const total = items.reduce((sum, item) => sum + item.tracks, 0);
  if (total <= 0) return [];
  return items
    .filter((item) => item.tracks > 0)
    .map((item) => ({
      ...item,
      width: Math.max(0.75, (item.tracks / total) * 100),
    }));
}

/** Share of the library that is lossless (DSD, Hi-Res and CD quality). */
export function losslessShare(tiers: Breakdown[]): number {
  const total = tiers.reduce((sum, tier) => sum + tier.tracks, 0);
  const lossless = tiers
    .filter((tier) => ["dsd", "hi_res", "lossless"].includes(tier.key))
    .reduce((sum, tier) => sum + tier.tracks, 0);
  return percent(lossless, total);
}

export type SkippedSection = {
  reason: SkipReason;
  label: string;
  items: SkippedFile[];
};

/** Skipped files grouped by reason, filtered by reason, text and ignore state. */
export function skippedSections(
  items: SkippedFile[],
  options: {
    reason?: SkipReason | null;
    query?: string;
    showIgnored?: boolean;
  } = {},
): SkippedSection[] {
  const query = options.query?.trim().toLowerCase() ?? "";
  const sections = new Map<SkipReason, SkippedSection>();
  for (const item of items) {
    if (item.ignored && !options.showIgnored) continue;
    if (options.reason && item.reason !== options.reason) continue;
    if (
      query &&
      !item.path.toLowerCase().includes(query) &&
      !item.detail.toLowerCase().includes(query)
    ) {
      continue;
    }
    const section = sections.get(item.reason) ?? {
      reason: item.reason,
      label: item.label,
      items: [],
    };
    section.items.push(item);
    sections.set(item.reason, section);
  }
  return [...sections.values()].sort((a, b) => b.items.length - a.items.length);
}

/** One sentence on why a reason matters, shown above its section. */
export const skipReasonHelp: Record<SkipReason, string> = {
  corrupt:
    "The file's audio stream could not be read, so it can't be played reliably.",
  tags_unreadable:
    "These play, but their names come from the file path because the tags are broken.",
  no_audio: "The container holds no measurable audio.",
  empty_file: "Zero-byte files, usually interrupted copies or downloads.",
  io_failure: "The server is not allowed to open these files.",
  unreadable_folder:
    "Folders the scan could not list; everything inside is missing from your library.",
  not_yet_imported: "New music that arrived after the last scan.",
  unsupported_format: "Audio in formats Parson does not decode.",
  drm_protected: "Copy-protected purchases that no other player can decode.",
  video_file: "Video files in your music folders.",
  disc_image: "Whole discs stored as one image or one long file.",
  playlist_file:
    "Playlists are imported only when you ask, so a scan never changes your playlists.",
  unreadable_image: "Cover art Parson can't use.",
};

export function duplicateKindLabel(kind: DuplicateGroup["kind"]): string {
  switch (kind) {
    case "same_album":
      return "Same album, several copies";
    case "loose":
      return "Loose copy of an album track";
    default:
      return "On several albums";
  }
}

export function duplicateEvidenceLabel(group: DuplicateGroup): string {
  const { evidence } = group;
  if (evidence.fingerprint_similarity !== null) {
    return `Sounds identical · ${Math.round(evidence.fingerprint_similarity * 100)}% fingerprint match`;
  }
  if (evidence.isrc) return "Same ISRC recording code";
  return evidence.duration_spread > 0
    ? `Same title and artist · lengths within ${evidence.duration_spread.toFixed(1)} s`
    : "Same title, artist and length";
}

export function runningJob(
  jobs: ToolJob[] | undefined,
  kind: ToolJobKind,
): ToolJob | undefined {
  return jobs?.find((job) => job.kind === kind && job.state === "running");
}

/** Jobs that just finished (were running in `previous`, are not now). */
export function finishedJobs(previous: ToolJob[], next: ToolJob[]): ToolJob[] {
  const running = new Set(
    previous.filter((job) => job.state === "running").map((job) => job.id),
  );
  return next.filter((job) => running.has(job.id) && job.state !== "running");
}

export function jobProgressLabel(job: ToolJob): string {
  if (job.state !== "running") return job.message || job.title;
  if (job.total > 0) {
    return `${formatCount(Math.min(job.done, job.total))} of ${formatCount(job.total)}`;
  }
  if (job.done > 0) return `${formatCount(job.done)} checked`;
  return "Starting…";
}

/** Moves `from` to `to` in a copy of `items`. */
export function moveItem<T>(items: T[], from: number, to: number): T[] {
  if (
    from === to ||
    from < 0 ||
    to < 0 ||
    from >= items.length ||
    to >= items.length
  ) {
    return items;
  }
  const next = [...items];
  const [moved] = next.splice(from, 1);
  if (moved === undefined) return items;
  next.splice(to, 0, moved);
  return next;
}

export const albumTypeLabels = {
  album: "Album",
  ep: "EP",
  single: "Single",
  compilation: "Compilation",
  live: "Live",
  soundtrack: "Soundtrack",
} as const;

/** Text on the button that forgets missing files. */
export function forgetButtonLabel(count: number): string {
  return count === 1
    ? "Forget 1 missing file"
    : `Forget ${count} missing files`;
}

/** Server timestamps are SQLite UTC (`2026-10-27 01:23:45`). */
export function parseServerTime(value: string | null | undefined): Date | null {
  if (!value) return null;
  const iso = value.includes("T") ? value : value.replace(" ", "T");
  const date = new Date(/[zZ]|[+-]\d\d:?\d\d$/.test(iso) ? iso : `${iso}Z`);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatServerTime(value: string | null | undefined): string {
  const date = parseServerTime(value);
  return date
    ? date.toLocaleString(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      })
    : "";
}
