import type {
  EnrichmentStageStatus,
  EnrichmentTaskState,
} from "../domain/types";

/**
 * Up to two initials for a person or group, used when there is no photo:
 * "Ludwig van Beethoven" -> "LB", "Wiener Philharmoniker" -> "WP",
 * "Björk" -> "B". Particles such as "van", "von" and "de" are skipped.
 */
export function personInitials(name: string) {
  const particles = new Set([
    "van",
    "von",
    "der",
    "den",
    "de",
    "da",
    "di",
    "du",
    "la",
    "le",
    "the",
    "and",
    "&",
  ]);
  const words = name
    .replace(/\([^)]*\)/g, " ")
    .split(/[\s\-–,./]+/)
    .filter(Boolean);
  const meaningful = words.filter(
    (word, index) => index === 0 || !particles.has(word.toLowerCase()),
  );
  const letters = (meaningful.length ? meaningful : words)
    .map((word) => Array.from(word).find((char) => /\p{L}|\p{N}/u.test(char)))
    .filter((char): char is string => Boolean(char));
  if (!letters.length) return "?";
  const picked =
    letters.length === 1 ? letters : [letters[0], letters[letters.length - 1]];
  return picked.join("").toLocaleUpperCase();
}

/** A stable hue (0-359) for an initials avatar, so a person keeps a color. */
export function personHue(name: string) {
  let hash = 0;
  for (const char of name.toLowerCase()) {
    hash = (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0;
  }
  return hash % 360;
}

type WaitingText = { artist: string; album: string };

const MUSICBRAINZ_WAITING: WaitingText = {
  artist: "Looking this artist up on MusicBrainz.",
  album: "Looking this album up on MusicBrainz.",
};

const STAGE_WAITING: Record<string, WaitingText> = {
  musicbrainz: MUSICBRAINZ_WAITING,
  wikidata: {
    artist: "Finding this artist on Wikidata.",
    album: "Finding this album on Wikidata.",
  },
  wikipedia: {
    artist: "Fetching the biography from Wikipedia.",
    album: "Fetching the background and reception from Wikipedia.",
  },
  commons: {
    artist: "Fetching the photo from Wikimedia Commons.",
    album: "Fetching images from Wikimedia Commons.",
  },
};

/**
 * What an entity's lookup is waiting for, or null when nothing is pending.
 * `stages` (from the enrichment overview, admins only) adds when a busy
 * host will be asked again.
 */
export function enrichmentWaitingLabel(
  task: EnrichmentTaskState | null | undefined,
  kind: "artist" | "album",
  stages?: readonly EnrichmentStageStatus[],
) {
  if (task?.status !== "pending") return null;
  const stage = task.stage ?? "musicbrainz";
  const label = (STAGE_WAITING[stage] ?? MUSICBRAINZ_WAITING)[kind];
  const paused = stages?.find((entry) => entry.stage === stage)?.paused_seconds;
  if (!paused) return label;
  const minutes = Math.max(1, Math.round(paused / 60));
  return `${label.replace(/\.$/, "")}; the service is busy, retrying in ${minutes} min.`;
}

export const ENRICHMENT_STAGE_LABELS: Record<string, string> = {
  musicbrainz: "MusicBrainz (matches and credits)",
  wikidata: "Wikidata (links and dates)",
  wikipedia: "Wikipedia (biographies and reviews)",
  commons: "Wikimedia Commons (artist photos)",
  relations: "Wikidata Query Service (influences and followers)",
  coverart: "Cover Art Archive (album covers)",
};

/** One line per stage queue for the settings page. */
export function enrichmentStageLine(stage: EnrichmentStageStatus) {
  const parts = [
    stage.counts.pending
      ? `${stage.counts.pending.toLocaleString()} queued`
      : null,
    `${stage.counts.done.toLocaleString()} done`,
    stage.counts.failed
      ? `${stage.counts.failed.toLocaleString()} failed`
      : null,
    stage.paused_seconds > 0
      ? `busy, retrying in ${Math.max(1, Math.round(stage.paused_seconds / 60))} min`
      : null,
  ].filter(Boolean);
  return {
    label: ENRICHMENT_STAGE_LABELS[stage.stage] ?? stage.host,
    detail: parts.join(" · "),
    busy: stage.paused_seconds > 0,
  };
}
