/**
 * Crossfade and volume leveling for this app's own player (the zone's
 * settings decide both). Pure helpers, so the timing and curves are tested
 * without a browser.
 */

/** The next track starts this long before the current one ends, at most. */
export function shouldStartCrossfade({
  seconds,
  currentTime,
  duration,
  repeatOne,
  hasNext,
  alreadyStarted,
}: {
  seconds: number;
  currentTime: number;
  duration: number;
  repeatOne: boolean;
  hasNext: boolean;
  alreadyStarted: boolean;
}): boolean {
  if (seconds <= 0 || repeatOne || !hasNext || alreadyStarted) return false;
  if (!Number.isFinite(duration) || duration <= 0) return false;
  // Very short tracks never crossfade over more than a third of their length.
  const window = Math.min(seconds, duration / 3);
  const remaining = duration - currentTime;
  return remaining > 0.05 && remaining <= window;
}

/** Equal-power gains at `progress` (0..1) through a crossfade. */
export function crossfadeGains(progress: number): {
  outgoing: number;
  incoming: number;
} {
  const t = Math.min(1, Math.max(0, progress));
  return {
    outgoing: Math.cos((t * Math.PI) / 2),
    incoming: Math.sin((t * Math.PI) / 2),
  };
}

/** Linear volume factor for a leveling gain in dB. */
export function levelingFactor(gainDb: number | null | undefined): number {
  return Number.isFinite(gainDb) ? 10 ** ((gainDb as number) / 20) : 1;
}

/**
 * The element volume for a user volume and a leveling factor. A browser
 * element cannot go above 1, so quiet tracks are raised only as far as the
 * user's volume leaves room.
 */
export function leveledVolume(userVolume: number, factor: number): number {
  return Math.min(1, Math.max(0, userVolume * factor));
}
