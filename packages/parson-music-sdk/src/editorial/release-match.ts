import type {
  AlbumAbout,
  ReleaseMatch,
  ReleaseMatchCheck,
  ReleaseMatchSignal,
} from "../domain/types";

function plural(count: number, word: string) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

/** The release check of an album that failed it, if any. */
export function unverifiedMatch(
  about: Pick<AlbumAbout, "release_match"> | null | undefined,
): ReleaseMatchCheck | null {
  const check = about?.release_match;
  return check?.status === "unverified" ? check : null;
}

/** "1957 · Single · 2 tracks" for the release a check is about. */
export function releaseMatchSummary(check: ReleaseMatchCheck) {
  const year = (check.release_date ?? check.first_release_date)?.slice(0, 4);
  return [
    year,
    check.release_type,
    check.release_track_count != null &&
      plural(check.release_track_count, "track"),
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * What an unverified match means for the album, in one sentence a
 * listener can act on.
 */
export const UNVERIFIED_MATCH_TEXT =
  "The closest MusicBrainz release does not line up with your files, so " +
  "Mujin Music kept your tags and did not use its cover, performers, works or " +
  "release type.";

/** Evidence against first, then evidence for, then what could not be told. */
export function releaseMatchSignals(
  check: ReleaseMatchCheck,
): ReleaseMatchSignal[] {
  const rank = { mismatch: 0, partial: 1, match: 2, unknown: 3 } as const;
  return [...(check.signals ?? [])].sort(
    (left, right) => (rank[left.outcome] ?? 4) - (rank[right.outcome] ?? 4),
  );
}

/** The "Matched" fact on an album page. */
export function releaseMatchFact(
  about: Pick<AlbumAbout, "release_match" | "identification"> | undefined,
): string | null {
  const check = about?.release_match;
  if (about?.identification || check?.method === "identified") {
    return "By hand on MusicBrainz";
  }
  if (!check) return null;
  if (check.method === "tagged") return "Release id in your tags";
  if (check.status === "unverified") return "Unverified";
  const signals = (check.signals ?? []).filter(
    (signal) => signal.outcome !== "unknown",
  );
  const agreed = signals.filter((signal) => signal.outcome === "match");
  return signals.length
    ? `Automatically · ${agreed.length} of ${signals.length} checks agree`
    : "Automatically";
}

/** Whether the release's year disagrees with the album's tags. */
export function releaseYearDiffers(
  about: Pick<AlbumAbout, "release_match"> | null | undefined,
) {
  return (about?.release_match?.signals ?? []).some(
    (signal) => signal.key === "year" && signal.outcome === "mismatch",
  );
}

/**
 * The warning "Identify album" shows before a release with a different
 * number of tracks is chosen; `null` when the counts agree or are unknown.
 */
export function trackCountWarning(
  release: Pick<ReleaseMatch, "track_count">,
  localTrackCount: number,
): string | null {
  if (release.track_count == null || release.track_count === localTrackCount) {
    return null;
  }
  return (
    `This release has ${plural(release.track_count, "track")}; your album ` +
    `has ${localTrackCount}. Tracks are paired by title, so some may get ` +
    "the wrong credits or piece."
  );
}
