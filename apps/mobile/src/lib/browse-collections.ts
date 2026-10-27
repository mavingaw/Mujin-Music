import {
  TAG_COLORS,
  groupHistoryByDay,
  formatListeningTime,
  type HistoryEntry,
  type ListenLaterItem,
  type TagColor,
} from "@parson/music-sdk";

/** Tag colours as hex for React Native (the web uses Tailwind tokens). */
export const TAG_HEX: Record<TagColor, string> = {
  slate: "#cbd5e1",
  red: "#f87171",
  orange: "#fb923c",
  amber: "#fcd34d",
  lime: "#bef264",
  green: "#34d399",
  teal: "#5eead4",
  sky: "#38bdf8",
  blue: "#60a5fa",
  violet: "#a78bfa",
  fuchsia: "#e879f9",
  pink: "#f472b6",
};

/** The same stable fallback colour the web picks for uncoloured tags. */
export function tagHex(tag: { name: string; color?: TagColor | null }) {
  if (tag.color && tag.color in TAG_HEX) return TAG_HEX[tag.color];
  let hash = 0;
  for (const character of tag.name.toLowerCase()) {
    hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  }
  return TAG_HEX[TAG_COLORS[hash % TAG_COLORS.length] ?? "violet"];
}

export interface HistorySection {
  key: string;
  title: string;
  subtitle: string;
  data: HistoryEntry[];
}

/** History grouped by local day for a SectionList. */
export function historySections(
  entries: HistoryEntry[],
  now: Date = new Date(),
): HistorySection[] {
  return groupHistoryByDay(entries, now).map((day) => ({
    key: day.key,
    title: day.label,
    subtitle: `${day.items.length} ${day.items.length === 1 ? "play" : "plays"} · ${formatListeningTime(day.listenedSeconds)}`,
    data: day.items,
  }));
}

export interface ListenLaterRow {
  key: string;
  item: ListenLaterItem;
  title: string;
  subtitle: string;
  artwork?: string | null;
  round: boolean;
}

/** Display rows for Listen Later, skipping items without library data. */
export function listenLaterRows(items: ListenLaterItem[]): ListenLaterRow[] {
  return items.flatMap((item): ListenLaterRow[] => {
    if (item.album) {
      return [
        {
          key: `album:${item.entityId}`,
          item,
          title: item.album.name,
          subtitle: ["Album", item.album.artistName, item.album.releaseYear]
            .filter(Boolean)
            .join(" · "),
          artwork: item.album.coverPath,
          round: false,
        },
      ];
    }
    if (item.song) {
      return [
        {
          key: `track:${item.entityId}`,
          item,
          title: item.song.name,
          subtitle: ["Track", item.song.artistName, item.song.albumName]
            .filter(Boolean)
            .join(" · "),
          artwork: item.song.coverPath,
          round: false,
        },
      ];
    }
    if (item.artist) {
      return [
        {
          key: `artist:${item.entityId}`,
          item,
          title: item.artist.name,
          subtitle: `Artist · ${item.artist.albumCount} ${
            item.artist.albumCount === 1 ? "album" : "albums"
          }`,
          artwork: item.artist.artworkPath,
          round: true,
        },
      ];
    }
    return [];
  });
}
