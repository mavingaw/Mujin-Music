import type {
  DiscographyFocusAlbum,
  FocusChip,
  FocusGroup,
  FocusGroupKey,
  FocusSummary,
} from "../domain/types";

/** Facet groups in display order. */
export const FOCUS_GROUPS: readonly FocusGroupKey[] = [
  "role",
  "performer",
  "conductor",
  "ensemble",
  "soloist",
  "instrument",
  "composer",
  "form",
  "period",
  "quality",
  "bit_depth",
  "sample_rate",
  "source",
];

/** Chosen values per group. Values in one group widen, groups narrow. */
export type FocusSelection = Partial<Record<FocusGroupKey, string[]>>;

const isGroup = (key: string): key is FocusGroupKey =>
  (FOCUS_GROUPS as readonly string[]).includes(key);

/** Reads a selection from URL search params (`conductor=…&bit_depth=24`),
 * ignoring every other parameter. */
export function focusSelectionFromParams(
  params: URLSearchParams | null | undefined,
): FocusSelection {
  const selection: FocusSelection = {};
  if (!params) return selection;
  for (const [key, value] of params.entries()) {
    const trimmed = value.trim();
    if (!isGroup(key) || !trimmed) continue;
    const values = selection[key] ?? [];
    if (!values.includes(trimmed)) values.push(trimmed);
    selection[key] = values;
  }
  return selection;
}

/** Writes a selection into URL search params, replacing any earlier one
 * and keeping every other parameter. */
export function focusSelectionToParams(
  selection: FocusSelection,
  base?: URLSearchParams | string,
): URLSearchParams {
  const params = new URLSearchParams(base);
  for (const group of FOCUS_GROUPS) params.delete(group);
  for (const group of FOCUS_GROUPS) {
    for (const value of selection[group] ?? []) params.append(group, value);
  }
  return params;
}

/** The query string the server reads, in a stable order (a cache key). */
export function focusQueryString(selection: FocusSelection): string {
  return focusSelectionToParams(selection).toString();
}

/** Chooses a value, or un-chooses it when it is already chosen. */
export function toggleFocusValue(
  selection: FocusSelection,
  group: FocusGroupKey,
  key: string,
): FocusSelection {
  const values = selection[group] ?? [];
  const next = values.includes(key)
    ? values.filter((value) => value !== key)
    : [...values, key];
  const result: FocusSelection = { ...selection };
  if (next.length) result[group] = next;
  else delete result[group];
  return result;
}

/** Removes one chosen value given as `group=value` (a summary's
 * `selected` key). */
export function removeFocusChip(
  selection: FocusSelection,
  chipKey: string,
): FocusSelection {
  const split = chipKey.indexOf("=");
  if (split <= 0) return selection;
  const group = chipKey.slice(0, split);
  if (!isGroup(group)) return selection;
  const key = chipKey.slice(split + 1);
  if (!(selection[group] ?? []).includes(key)) return selection;
  return toggleFocusValue(selection, group, key);
}

/** How many values are chosen in all. */
export function focusSelectionCount(selection: FocusSelection): number {
  return FOCUS_GROUPS.reduce(
    (count, group) => count + (selection[group]?.length ?? 0),
    0,
  );
}

/** The groups shown first when there are many; the rest wait behind
 * "More filters". */
const PRIMARY_GROUPS: readonly FocusGroupKey[] = [
  "role",
  "performer",
  "conductor",
  "ensemble",
  "soloist",
  "composer",
  "quality",
  "bit_depth",
  "sample_rate",
];

/** At most this many groups show before "More filters". */
const MAX_PRIMARY = 8;

/** Splits groups into those always shown and those behind "More". A group
 * with a chosen value is always shown; a short list is shown whole. */
export function splitFocusGroups(groups: FocusGroup[]): {
  primary: FocusGroup[];
  more: FocusGroup[];
} {
  if (groups.length <= MAX_PRIMARY) return { primary: groups, more: [] };
  const primary: FocusGroup[] = [];
  const more: FocusGroup[] = [];
  for (const group of groups) {
    const chosen = group.values.some((chip) => chip.selected);
    if (chosen || PRIMARY_GROUPS.includes(group.key)) primary.push(group);
    else more.push(group);
  }
  return { primary, more };
}

/** A one-line summary of what is chosen: "Karajan · Philharmonia · 24-bit". */
export function describeFocus(summary: FocusSummary | null | undefined) {
  if (!summary?.selected.length) return "";
  return summary.selected.map((chip) => chip.label).join(" · ");
}

/** "3 of 12 albums". */
export function focusResultLabel(
  summary: FocusSummary | null | undefined,
  noun: { one: string; other: string } = { one: "album", other: "albums" },
) {
  if (!summary) return "";
  const word = summary.total === 1 ? noun.one : noun.other;
  return summary.matched === summary.total
    ? `${summary.total} ${word}`
    : `${summary.matched} of ${summary.total} ${word}`;
}

const ROLE_LABELS: Record<string, string> = {
  main: "Main artist",
  featured: "Featured",
  composer: "Composer",
  lyricist: "Lyricist",
  arranger: "Arranger",
  conductor: "Conductor",
  performer: "Performer",
  ensemble: "Ensemble",
  producer: "Producer",
  remixer: "Remixer",
  mixer: "Mixing",
  engineer: "Engineer",
};

/** "Composer", "Main artist", ... */
export function focusRoleLabel(role: string) {
  return (
    ROLE_LABELS[role] ??
    (role ? role.charAt(0).toUpperCase() + role.slice(1) : role)
  );
}

/** Whether a chip names a person with a page. */
export function focusChipPerson(chip: FocusChip) {
  return chip.artist_id ?? null;
}

export interface DiscographySection {
  key: string;
  title: string;
  albums: DiscographyFocusAlbum[];
}

const RELEASE_SECTIONS: { types: string[]; key: string; title: string }[] = [
  { types: ["album", ""], key: "albums", title: "Albums" },
  { types: ["ep", "single"], key: "singles", title: "Singles & EPs" },
  { types: ["compilation"], key: "compilations", title: "Compilations" },
  { types: ["live"], key: "live", title: "Live" },
  { types: ["soundtrack"], key: "soundtracks", title: "Soundtracks" },
];

const ROLE_SECTIONS: { roles: string[]; key: string; title: string }[] = [
  { roles: ["conductor"], key: "conducted", title: "As conductor" },
  {
    roles: ["featured", "performer", "ensemble"],
    key: "appears",
    title: "Appears on",
  },
  {
    roles: ["composer", "lyricist", "arranger"],
    key: "composed",
    title: "Recordings of their music",
  },
  {
    roles: ["producer", "remixer", "mixer", "engineer"],
    key: "production",
    title: "Production & engineering",
  },
];

/**
 * Sections a discography the way a reader scans it: the person's own
 * releases by type, then what they conducted, appear on, wrote, and
 * produced. Each album sits in its most important section only.
 */
export function discographySections(
  albums: readonly DiscographyFocusAlbum[],
): DiscographySection[] {
  const sections = new Map<string, DiscographySection>();
  const order: string[] = [];
  const push = (key: string, title: string, album: DiscographyFocusAlbum) => {
    let section = sections.get(key);
    if (!section) {
      section = { key, title, albums: [] };
      sections.set(key, section);
    }
    section.albums.push(album);
  };
  for (const album of albums) {
    if (album.roles.includes("main")) {
      const type = (album.album.primary_type ?? "").toLowerCase();
      const release = RELEASE_SECTIONS.find((entry) =>
        entry.types.includes(type),
      ) ?? {
        key: "other",
        title: "Other releases",
      };
      push(release.key, release.title, album);
      continue;
    }
    const role = ROLE_SECTIONS.find((entry) =>
      entry.roles.some((candidate) => album.roles.includes(candidate)),
    ) ?? { key: "other-credits", title: "Other credits" };
    push(role.key, role.title, album);
  }
  for (const entry of [
    ...RELEASE_SECTIONS,
    { key: "other", title: "" },
    ...ROLE_SECTIONS,
    { key: "other-credits", title: "" },
  ]) {
    if (sections.has(entry.key)) order.push(entry.key);
  }
  return order.map((key) => sections.get(key)!);
}

/** A compact resolution badge: "24/96", "16/44.1", "MP3". */
export function qualityBadge(
  album: Pick<DiscographyFocusAlbum, "format" | "quality">,
) {
  if (!album.format) return null;
  const resolution = parseResolution(album.format);
  if (resolution && album.quality !== "lossy")
    return `${resolution.bits}/${resolution.rate}`;
  return album.format.split(" ")[0] ?? album.format;
}

const isDigit = (char: string | undefined) =>
  char !== undefined && char >= "0" && char <= "9";

/**
 * The first "<bits>-bit / <rate> kHz" in a format line, scanned in linear
 * time.
 */
function parseResolution(format: string) {
  const separator = "-bit / ";
  for (
    let at = format.indexOf(separator);
    at >= 0;
    at = format.indexOf(separator, at + 1)
  ) {
    let bitsStart = at;
    while (isDigit(format[bitsStart - 1])) bitsStart -= 1;
    if (bitsStart === at) continue;
    const rateStart = at + separator.length;
    let rateEnd = rateStart;
    while (isDigit(format[rateEnd]) || format[rateEnd] === ".") rateEnd += 1;
    if (rateEnd === rateStart || !format.startsWith(" kHz", rateEnd)) continue;
    return {
      bits: format.slice(bitsStart, at),
      rate: format.slice(rateStart, rateEnd),
    };
  }
  return null;
}
