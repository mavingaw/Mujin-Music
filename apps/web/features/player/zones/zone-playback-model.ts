import type { Zone } from "@parson/music-sdk";

/** Where a library Play / Play next / Add to queue action goes. */
export type PlaybackTarget =
  | { kind: "local" }
  | {
      kind: "zone";
      zoneId: string;
      /** Shown in toasts; the id while the zone list is still loading. */
      name: string;
      /** Takes a new queue (album, track and playlist Play). */
      canLoad: boolean;
      /** Takes Play next and Add to queue. */
      canQueue: boolean;
    };

export interface PlaybackTargetInput {
  /** The zone the person picked (stored), before the zone list resolves it. */
  requestedZoneId: string | null;
  localZoneId: string | null;
  zones: readonly Zone[];
  /** False until the first zone list arrived. */
  loaded: boolean;
}

/**
 * Library actions follow the zone picker: "This device" plays in the
 * browser, any other zone gets the command on the server. A picked zone
 * keeps its commands while the zone list is still loading (so a fresh page
 * never plays in the browser by mistake); a zone that is gone falls back to
 * this device.
 */
export function resolvePlaybackTarget({
  requestedZoneId,
  localZoneId,
  zones,
  loaded,
}: PlaybackTargetInput): PlaybackTarget {
  if (!requestedZoneId || requestedZoneId === localZoneId) {
    return { kind: "local" };
  }
  const zone = zones.find((candidate) => candidate.id === requestedZoneId);
  if (!zone) {
    if (loaded) return { kind: "local" };
    return {
      kind: "zone",
      zoneId: requestedZoneId,
      name: "the selected zone",
      canLoad: true,
      canQueue: true,
    };
  }
  return {
    kind: "zone",
    zoneId: zone.id,
    name: zone.name,
    canLoad: zone.capabilities.transfer_target,
    canQueue: zone.capabilities.queue_edit || zone.queue.song_ids.length === 0,
  };
}

/** The toast after a remote Play next / Add to queue. */
export function queuedOnZoneMessage(
  what: string,
  zoneName: string,
  next: boolean,
): string {
  return next
    ? `${what} will play next on ${zoneName}`
    : `Added ${what} to the queue on ${zoneName}`;
}

/** A readable reason a zone refused a library action. */
export function zoneActionError(error: unknown, zoneName: string): string {
  const code =
    (error as { response?: { data?: { error?: string } } })?.response?.data
      ?.error ?? (error instanceof Error ? error.message : "");
  switch (code) {
    case "zone_cannot_receive_transfer":
      return `${zoneName} plays what the phone that started it sends; start it from there.`;
    case "edit_the_cast_queue_on_the_phone":
      return `Edit ${zoneName}'s queue on the phone that started the cast.`;
    case "queue_full":
      return `${zoneName}'s queue is full (500 songs).`;
    case "zone_not_found":
      return `${zoneName} is no longer available.`;
    default:
      return `${zoneName} did not take that${code ? ` (${code})` : ""}.`;
  }
}
