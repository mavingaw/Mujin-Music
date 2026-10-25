import type { PlaybackEventType } from "../resources/users";
import type { RadioSeed, ResolvedRadioSeed } from "../resources/playback";

/** Seconds before a Radio pick starts when the "Coming up" notice shows. */
export const radioComingUpWindowSeconds = 20;

/** Top up the Radio section when fewer upcoming picks than this remain. */
export const radioMinimumUpcomingPicks = 3;

export type RadioSkipReasonId = "dislike" | "not_now" | "doesnt_fit";

export type RadioSkipReason = {
  id: RadioSkipReasonId;
  label: string;
  event: PlaybackEventType;
};

/** The optional "Tell us why" answers after skipping a Radio pick. */
export const radioSkipReasons: readonly RadioSkipReason[] = [
  { id: "dislike", label: "I don’t like this track", event: "disliked" },
  { id: "not_now", label: "Not right now", event: "skip_not_now" },
  {
    id: "doesnt_fit",
    label: "Doesn’t fit this station",
    event: "skip_doesnt_fit",
  },
];

export function radioSkipReasonEvent(id: RadioSkipReasonId): PlaybackEventType {
  return (
    radioSkipReasons.find((reason) => reason.id === id)?.event ?? "early_skip"
  );
}

type Origin = "manual" | "generated";

/**
 * Index where the visible Radio section starts: the first Radio pick after
 * the current item, once every manually queued item has played. Returns -1
 * when nothing upcoming is a Radio pick.
 */
export function radioSectionStart(
  origins: readonly Origin[],
  currentIndex: number,
): number {
  const from = Math.max(0, currentIndex + 1);
  let start = -1;
  for (let index = origins.length - 1; index >= from; index -= 1) {
    if (origins[index] !== "generated") break;
    start = index;
  }
  return start;
}

/** Number of Radio picks still to come after the current item. */
export function upcomingRadioPicks(
  origins: readonly Origin[],
  currentIndex: number,
): number {
  const start = radioSectionStart(origins, currentIndex);
  return start < 0 ? 0 : origins.length - start;
}

/**
 * Seconds until the next item starts when it is a Radio pick inside the
 * notice window, otherwise null.
 */
export function radioComingUpIn(options: {
  currentTime: number;
  duration: number;
  nextIsRadioPick: boolean;
  looping?: boolean;
  windowSeconds?: number;
}): number | null {
  const { currentTime, duration, nextIsRadioPick, looping } = options;
  const window = options.windowSeconds ?? radioComingUpWindowSeconds;
  if (!nextIsRadioPick || looping) return null;
  if (!Number.isFinite(duration) || duration <= window + 5) return null;
  if (!Number.isFinite(currentTime) || currentTime < 0) return null;
  const remaining = duration - currentTime;
  return remaining > 0 && remaining <= window ? Math.ceil(remaining) : null;
}

/** Radio does not run alongside a repeat mode (Roon behaves the same way). */
export function shouldContinueWithRadio(options: {
  enabled: boolean;
  repeating: boolean;
}): boolean {
  return options.enabled && !options.repeating;
}

/** Heading for the Radio section, e.g. "Playing Miles Davis radio". */
export function radioStationLabel(
  seed?: Pick<ResolvedRadioSeed, "kind" | "name"> | null,
): string {
  if (!seed?.name) return "Based on your queue";
  return seed.kind === "song"
    ? `Based on “${seed.name}”`
    : `Playing ${seed.name} radio`;
}

/**
 * The queue as Radio's play history: each song once, at its most recent
 * position, oldest first, capped to the newest `limit`. Radio uses the order
 * to keep repeats a while away once the library is used up.
 */
export function radioQueueHistory(
  songIds: Iterable<string | null | undefined>,
  limit = 500,
): string[] {
  const latest = new Map<string, number>();
  let position = 0;
  for (const id of songIds) {
    if (id) latest.set(id, position);
    position += 1;
  }
  return [...latest.entries()]
    .sort((left, right) => left[1] - right[1])
    .map(([id]) => id)
    .slice(-Math.max(0, limit));
}

/** Stable key for comparing seeds. */
export function radioSeedKey(seed?: RadioSeed | null): string {
  return seed?.id ? `${seed.kind}:${seed.id}` : "";
}
