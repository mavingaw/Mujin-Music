export function boundedMediaPosition(
  value: number | string,
  duration?: number,
): number | null {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return null;

  const nonNegative = Math.max(0, numeric);
  return Number.isFinite(duration) && Number(duration) > 0
    ? Math.min(nonNegative, Number(duration))
    : nonNegative;
}

export function setMediaPosition(
  element: Pick<HTMLMediaElement, "currentTime"> | null,
  value: number | string,
  duration?: number,
): number | null {
  const bounded = boundedMediaPosition(value, duration);
  if (bounded === null) return null;
  if (element) {
    try {
      element.currentTime = bounded;
    } catch {
      return null;
    }
  }
  return bounded;
}

export function boundedVolume(value: number | string): number | null {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return null;
  return Math.max(0, Math.min(1, numeric / 100));
}

export function isCurrentTrackGeneration(
  requestedGeneration: number,
  currentGeneration: number,
  requestedSongId?: string,
  currentSongId?: string,
) {
  return (
    requestedGeneration === currentGeneration &&
    (requestedSongId === undefined || requestedSongId === currentSongId)
  );
}

export function queueIndexForPersistedPosition(
  positions: Array<number | null>,
  persistedPosition: number,
): number {
  if (!positions.length) return 0;
  const exact = positions.findIndex(
    (position) => position === persistedPosition,
  );
  if (exact >= 0) return exact;
  const next = positions.findIndex(
    (position) => position !== null && position > persistedPosition,
  );
  return next >= 0 ? next : positions.length - 1;
}

/**
 * The length to show for the playing track. A file the server converts
 * while it plays (ALAC to FLAC, DSD to PCM) arrives as a stream with no
 * length, so the media element reports Infinity or NaN, or only the part
 * received so far. The library already knows the track's length: use it
 * then, so the total time and seek bar are right on the first play.
 */
export function effectiveDuration(
  mediaDuration: number | undefined,
  knownDuration: number | undefined,
): number {
  const media = Number(mediaDuration);
  const known = Number(knownDuration);
  const hasKnown = Number.isFinite(known) && known > 0;
  if (!Number.isFinite(media) || media <= 0) return hasKnown ? known : 0;
  // A partly received stream reports what has arrived so far.
  if (hasKnown && media < known * 0.5) return known;
  return media;
}
