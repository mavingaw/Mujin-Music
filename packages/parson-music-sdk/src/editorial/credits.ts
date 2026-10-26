import type {
  AboutCredit,
  ArtistFacts,
  AudioProperties,
  Credit,
  CreditRoleRef,
} from "../domain/types";

/** Display order for credit roles, matching the server's canonical order. */
export const CREDIT_ROLE_ORDER = [
  "composer",
  "lyricist",
  "arranger",
  "conductor",
  "performer",
  "ensemble",
  "producer",
  "remixer",
  "mixer",
  "engineer",
] as const;

const ROLE_LABELS: Record<string, [singular: string, plural: string]> = {
  composer: ["Composer", "Composers"],
  lyricist: ["Lyricist", "Lyricists"],
  arranger: ["Arranger", "Arrangers"],
  conductor: ["Conductor", "Conductors"],
  performer: ["Performer", "Performers"],
  ensemble: ["Ensemble", "Ensembles"],
  producer: ["Producer", "Producers"],
  remixer: ["Remixer", "Remixers"],
  mixer: ["Mixing", "Mixing"],
  engineer: ["Engineer", "Engineers"],
};

const ROLE_VERBS: Record<string, string> = {
  composer: "Composed by",
  lyricist: "Lyrics by",
  arranger: "Arranged by",
  conductor: "Conducted by",
  performer: "Performed by",
  ensemble: "Performed by",
  producer: "Produced by",
  remixer: "Remixed by",
  mixer: "Mixed by",
  engineer: "Engineered by",
};

export function roleRank(role: string) {
  const index = (CREDIT_ROLE_ORDER as readonly string[]).indexOf(role);
  return index < 0 ? CREDIT_ROLE_ORDER.length : index;
}

function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

/** "Producer", "Piano", "Lead vocals"; instruments name performer credits. */
export function creditRoleLabel(
  role: string,
  instrument?: string | null,
  plural = false,
) {
  if (instrument) return capitalize(instrument);
  const labels = ROLE_LABELS[role];
  if (labels) return plural ? labels[1] : labels[0];
  return capitalize(role.replace(/[_-]+/g, " "));
}

export function roleVerb(role: string) {
  return ROLE_VERBS[role] ?? `${creditRoleLabel(role)}:`;
}

/** "Producer", "Composer & Producer", "Piano & Composer". */
export function creditRolesLabel(roles: readonly CreditRoleRef[]) {
  return [...roles]
    .sort((left, right) => roleRank(left.role) - roleRank(right.role))
    .map((role) => creditRoleLabel(role.role, role.instrument))
    .filter((label, index, all) => all.indexOf(label) === index)
    .join(" & ");
}

export interface CreditPerson {
  id: string | null;
  name: string;
}

export interface CreditGroup<T extends CreditPerson = CreditPerson> {
  key: string;
  label: string;
  credits: T[];
}

type GroupableCredit = {
  artist_id: string | null;
  name: string;
  role: string;
  instrument: string | null;
};

/**
 * Groups credits by role in display order. Performers are grouped per
 * instrument so "Piano", "Drums" and "Bass" read like liner notes.
 */
export function groupCredits<T extends GroupableCredit>(
  credits: readonly T[],
): CreditGroup<T & CreditPerson>[] {
  const groups = new Map<
    string,
    { rank: number; label: string; credits: (T & CreditPerson)[] }
  >();
  for (const credit of credits) {
    const key = credit.instrument
      ? `${credit.role}:${credit.instrument.toLowerCase()}`
      : credit.role;
    let group = groups.get(key);
    if (!group) {
      group = {
        rank: roleRank(credit.role),
        label: creditRoleLabel(credit.role, credit.instrument),
        credits: [],
      };
      groups.set(key, group);
    }
    if (
      !group.credits.some(
        (existing) =>
          existing.name === credit.name && existing.id === credit.artist_id,
      )
    ) {
      group.credits.push({ ...credit, id: credit.artist_id });
    }
  }
  return [...groups.entries()]
    .sort(
      ([leftKey, left], [rightKey, right]) =>
        left.rank - right.rank ||
        // Plain role before its instruments, then instruments by name.
        Number(leftKey.includes(":")) - Number(rightKey.includes(":")) ||
        left.label.localeCompare(right.label),
    )
    .map(([key, group]) => ({
      key,
      label:
        group.credits.length > 1 && !key.includes(":")
          ? creditRoleLabel(key, null, true)
          : group.label,
      credits: group.credits,
    }));
}

/** A track's credits as album-page credits (from tags only). */
export function tagCreditsAsAbout(
  trackId: string,
  credits: readonly Credit[] | undefined,
): AboutCredit[] {
  return (credits ?? []).map((credit) => ({
    artist_id: credit.artist_id,
    name: credit.name,
    role: credit.role,
    instrument: credit.instrument,
    track_ids: [trackId],
    sources: ["tags"],
  }));
}

/** Credits that apply to one track; release-wide credits are left out. */
export function creditsForTrack(
  credits: readonly AboutCredit[],
  trackId: string,
) {
  return credits.filter((credit) => credit.track_ids.includes(trackId));
}

/**
 * The line Roon shows under a track title: "Composed by A, B". Composers
 * win; otherwise lyricists, then nothing.
 */
export function trackCreditLine(credits: readonly GroupableCredit[]) {
  for (const role of ["composer", "lyricist"]) {
    const people = credits
      .filter((credit) => credit.role === role)
      .map((credit) => ({ id: credit.artist_id, name: credit.name }))
      .filter(
        (person, index, all) =>
          all.findIndex((other) => other.name === person.name) === index,
      );
    if (people.length) return { label: roleVerb(role), people };
  }
  return null;
}

/**
 * "All tracks", "Track 3" or "Tracks 1–3, 5" for a credit, using track
 * numbers from the album.
 */
export function creditTracksLabel(
  trackIds: readonly string[],
  tracks: readonly { id: string; track_number?: number }[],
) {
  if (!trackIds.length) return "All tracks";
  if (tracks.length > 1 && tracks.every((track) => trackIds.includes(track.id)))
    return "All tracks";
  const numbers = tracks
    .map((track, index) => ({
      id: track.id,
      number: track.track_number || index + 1,
    }))
    .filter((track) => trackIds.includes(track.id))
    .map((track) => track.number)
    .sort((left, right) => left - right);
  const [first] = numbers;
  if (first === undefined) return "";
  const ranges: string[] = [];
  let start = first;
  let previous = first;
  for (const number of [...numbers.slice(1), Number.NaN]) {
    if (number === previous + 1) {
      previous = number;
      continue;
    }
    ranges.push(
      start === previous
        ? `${start}`
        : previous === start + 1
          ? `${start}, ${previous}`
          : `${start}–${previous}`,
    );
    start = number;
    previous = number;
  }
  return `${numbers.length === 1 ? "Track" : "Tracks"} ${ranges.join(", ")}`;
}

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/** "1959", "August 1959" or "17 August 1959" from a partial ISO date. */
export function formatPartialDate(value?: string | null) {
  const match = /^(-?\d{1,4})(?:-(\d{2}))?(?:-(\d{2}))?/.exec(
    value?.trim() ?? "",
  );
  if (!match) return null;
  const [, year, month, day] = match;
  const monthName = month ? MONTHS[Number(month) - 1] : undefined;
  if (!monthName) return year;
  if (!day || day === "00") return `${monthName} ${year}`;
  return `${Number(day)} ${monthName} ${year}`;
}

/** Born/Died for people, Formed/Disbanded for groups. */
export function lifeSpanFacts(facts: ArtistFacts | null | undefined) {
  if (!facts) return [];
  const person = !facts.kind || facts.kind === "Person";
  const rows: { label: string; value: string }[] = [];
  const begin = formatPartialDate(facts.begin);
  const end = formatPartialDate(facts.end);
  const place = facts.begin_area ?? null;
  if (begin)
    rows.push({
      label: person ? "Born" : "Formed",
      value: place ? `${begin} in ${place}` : begin,
    });
  if (end) rows.push({ label: person ? "Died" : "Disbanded", value: end });
  if (facts.area && facts.area !== place)
    rows.push({ label: person ? "From" : "Origin", value: facts.area });
  return rows;
}

const CODEC_NAMES: Record<string, string> = {
  flac: "FLAC",
  alac: "ALAC",
  aac: "AAC",
  mp3: "MP3",
  vorbis: "Ogg Vorbis",
  opus: "Opus",
  pcm: "PCM",
  pcm_float: "PCM",
  wavpack: "WavPack",
  ape: "APE",
};

function formatRate(sampleRate: number) {
  // 44.1, 88.2 and 22.05 kHz, never rounded to a rate that does not exist.
  return `${Number((sampleRate / 1000).toFixed(2))} kHz`;
}

/** "FLAC 96 kHz 24-bit" or "MP3 320 kbps" for one file's properties. */
export function formatAudio(audio: AudioProperties | null | undefined) {
  if (!audio?.codec) return null;
  const codec = CODEC_NAMES[audio.codec] ?? audio.codec.toUpperCase();
  if (audio.lossless) {
    return [
      codec,
      audio.sample_rate ? formatRate(audio.sample_rate) : null,
      audio.bit_depth ? `${audio.bit_depth}-bit` : null,
    ]
      .filter(Boolean)
      .join(" ");
  }
  return [codec, audio.bitrate_kbps ? `${audio.bitrate_kbps} kbps` : null]
    .filter(Boolean)
    .join(" ");
}

/** The format most of an album's tracks share, or "Mixed formats". */
export function albumFormat(
  songs: readonly { audio?: AudioProperties | null }[],
) {
  const counts = new Map<string, number>();
  for (const song of songs) {
    const label = formatAudio(song.audio);
    if (label) counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  const [best] = [...counts.entries()].sort(
    (left, right) => right[1] - left[1],
  );
  if (!best) return null;
  return counts.size === 1 || best[1] * 2 > songs.length
    ? best[0]
    : "Mixed formats";
}

/** Splits stored editorial text into paragraphs. */
export function paragraphs(text: string) {
  return text
    .split(/\n{2,}/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean);
}
