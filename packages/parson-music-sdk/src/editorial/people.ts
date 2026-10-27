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
  const words = withoutParentheses(name)
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

/**
 * Muted Mujin Music tones for initials avatars ([lit stop, deep stop]):
 * copper, steel, sage, brass, clay, graphite.
 */
export const PERSON_TONES: readonly [string, string][] = [
  ["#8a5638", "#2a1a12"],
  ["#4f6d7d", "#18232a"],
  ["#5b7550", "#1a2217"],
  ["#8a6c43", "#261d12"],
  ["#7d5d57", "#251b19"],
  ["#55555d", "#18181b"],
];

/** A stable brand tone for a person without a photo. */
export function personTone(name: string): [string, string] {
  const [from, to] = PERSON_TONES[personHue(name) % PERSON_TONES.length]!;
  return [from, to];
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

/**
 * Skip reasons that describe the server's own bookkeeping rather than the
 * album: an item the in-memory library briefly missed while it reloaded.
 * They are never shown as "no confident match", since the item is present.
 */
const BOOKKEEPING_SKIPS = [
  "no longer in the library",
  "the library is still loading",
];

/**
 * Whether a skipped lookup really means "no confident MusicBrainz match",
 * and the reason to show beside it (null when there is none worth showing).
 */
export function enrichmentNoMatch(
  task: EnrichmentTaskState | null | undefined,
): { reason: string | null } | null {
  if (task?.status !== "skipped") return null;
  const reason = task.last_error?.trim() || null;
  if (reason && BOOKKEEPING_SKIPS.some((text) => reason.includes(text))) {
    return null;
  }
  return { reason };
}

export const ENRICHMENT_STAGE_LABELS: Record<string, string> = {
  musicbrainz: "MusicBrainz (matches and credits)",
  wikidata: "Wikidata (links and dates)",
  wikipedia: "Wikipedia (biographies and reviews)",
  commons: "Wikimedia Commons (artist photos)",
  relations: "Wikidata Query Service (influences and followers)",
  coverart: "Cover Art Archive (album covers)",
  catalogue: "MusicBrainz and Wikidata (composers' full catalogues)",
  sources: "Wikipedia (other editorial sources)",
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

/**
 * Replaces each "(…)" aside with a space, like `/\([^)]*\)/g`, in one pass:
 * an unclosed "(" and everything after it stay as written.
 */
function withoutParentheses(name: string) {
  let out = "";
  let at = 0;
  while (at < name.length) {
    const open = name.indexOf("(", at);
    if (open < 0) break;
    const close = name.indexOf(")", open + 1);
    if (close < 0) break;
    out += `${name.slice(at, open)} `;
    at = close + 1;
  }
  return out + name.slice(at);
}
