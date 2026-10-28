import {
  isUpgradeOpen,
  type UpgradeCandidate,
  type UpgradeReasonKind,
  type UpgradeRequest,
} from "@parson/music-sdk";

/**
 * Reason badge colours follow the signal-path light: lossy reasons sit with
 * the low-quality grey (the light dims rather than alarms; a fake lossless
 * file is a brighter grey so it still stands out), AI-enhanced takes the
 * enhanced blue, and CD quality the lossless copper it already is.
 */
export const reasonColors: Record<UpgradeReasonKind, string> = {
  lossy: "var(--color-quality-low)",
  low_bitrate: "var(--color-quality-low)",
  fake_lossless: "var(--color-mujin-text-secondary)",
  ai_enhanced_only: "var(--color-quality-enhanced)",
  cd_quality: "var(--color-mujin-copper)",
};

export type CandidateFilter = "all" | "waiting" | "requested" | "unfound";

export const candidateFilterLabels: Record<CandidateFilter, string> = {
  all: "All",
  waiting: "Not requested",
  requested: "On their way",
  unfound: "Not found",
};

const UNFOUND = new Set([
  "not_found",
  "failed",
  "no_match",
  "fake_lossless",
] as const);

export function isUnfound(candidate: UpgradeCandidate): boolean {
  const state = candidate.request?.state;
  return (
    (state !== undefined && UNFOUND.has(state as never)) ||
    candidate.search?.found === false
  );
}

export function matchesFilter(
  candidate: UpgradeCandidate,
  filter: CandidateFilter,
): boolean {
  const open = candidate.request
    ? isUpgradeOpen(candidate.request.state)
    : false;
  switch (filter) {
    case "all":
      return true;
    case "waiting":
      return !candidate.request || candidate.request.state === "cancelled";
    case "requested":
      return open;
    case "unfound":
      return isUnfound(candidate);
  }
}

/**
 * Badges for a row without saying the same thing twice: when the copy that
 * plays is AI-enhanced, its quality badge already says so, and the track
 * count only shows while some tracks are still the lossy originals.
 */
export function candidateBadges(candidate: UpgradeCandidate) {
  const enhancedQuality = candidate.quality.tier === "ai_enhanced";
  return {
    reasons: candidate.reasons.filter(
      (reason) => !(enhancedQuality && reason.kind === "ai_enhanced_only"),
    ),
    enhancedCount:
      candidate.enhanced_tracks > 0 &&
      candidate.enhanced_tracks < candidate.track_count
        ? `${candidate.enhanced_tracks} of ${candidate.track_count} AI-enhanced`
        : null,
  };
}

/** A request can be made unless one is already on its way. */
export function canRequest(candidate: UpgradeCandidate): boolean {
  return !candidate.request || !isUpgradeOpen(candidate.request.state);
}

/** Albums the bulk request would send, in list order. */
export function requestableSelection(
  candidates: UpgradeCandidate[],
  selected: ReadonlySet<string>,
): UpgradeCandidate[] {
  return candidates.filter(
    (candidate) => selected.has(candidate.album_id) && canRequest(candidate),
  );
}

/** Light colour of a request's state. */
export function requestColor(request: UpgradeRequest): string {
  switch (request.state) {
    case "upgraded":
      return "var(--color-quality-lossless)";
    case "fake_lossless":
    case "failed":
      return "var(--color-mujin-text-secondary)";
    case "no_match":
    case "not_found":
    case "cancelled":
      return "var(--color-mujin-text-tertiary)";
    default:
      return "var(--color-mujin-copper-hover)";
  }
}

/** Seconds between refreshes: quick while something moves. */
export function refreshInterval(
  candidates: UpgradeCandidate[],
  requests: UpgradeRequest[],
): number | false {
  const moving = requests.some((request) => isUpgradeOpen(request.state));
  const queued = candidates.some((candidate) => candidate.enhance_queued > 0);
  return moving || queued ? 4_000 : false;
}
