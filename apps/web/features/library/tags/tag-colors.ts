import { TAG_COLORS, type TagColor } from "@parson/music-sdk";

export interface TagTone {
  /** Small solid dot. */
  dot: string;
  /** Chip / pill. */
  chip: string;
  /** Text accent on dark surfaces. */
  text: string;
  /** Soft wash behind headers and tiles. */
  wash: string;
}

// Full class strings so Tailwind keeps them.
const TONES: Record<TagColor, TagTone> = {
  slate: {
    dot: "bg-slate-300",
    chip: "border-slate-300/25 bg-slate-300/10 text-slate-100",
    text: "text-slate-300",
    wash: "from-slate-400/30",
  },
  red: {
    dot: "bg-red-400",
    chip: "border-red-300/30 bg-red-400/12 text-red-100",
    text: "text-red-300",
    wash: "from-red-500/30",
  },
  orange: {
    dot: "bg-orange-400",
    chip: "border-orange-300/30 bg-orange-400/12 text-orange-100",
    text: "text-orange-300",
    wash: "from-orange-500/30",
  },
  amber: {
    dot: "bg-amber-300",
    chip: "border-amber-200/30 bg-amber-300/12 text-amber-100",
    text: "text-amber-200",
    wash: "from-amber-400/30",
  },
  lime: {
    dot: "bg-lime-300",
    chip: "border-lime-200/30 bg-lime-300/12 text-lime-100",
    text: "text-lime-200",
    wash: "from-lime-400/25",
  },
  green: {
    dot: "bg-emerald-400",
    chip: "border-emerald-300/30 bg-emerald-400/12 text-emerald-100",
    text: "text-emerald-300",
    wash: "from-emerald-500/30",
  },
  teal: {
    dot: "bg-teal-300",
    chip: "border-teal-200/30 bg-teal-300/12 text-teal-100",
    text: "text-teal-200",
    wash: "from-teal-400/30",
  },
  sky: {
    dot: "bg-sky-400",
    chip: "border-sky-300/30 bg-sky-400/12 text-sky-100",
    text: "text-sky-300",
    wash: "from-sky-500/30",
  },
  blue: {
    dot: "bg-blue-400",
    chip: "border-blue-300/30 bg-blue-400/12 text-blue-100",
    text: "text-blue-300",
    wash: "from-blue-500/30",
  },
  violet: {
    dot: "bg-violet-400",
    chip: "border-violet-300/30 bg-violet-400/12 text-violet-100",
    text: "text-violet-300",
    wash: "from-violet-500/30",
  },
  fuchsia: {
    dot: "bg-fuchsia-400",
    chip: "border-fuchsia-300/30 bg-fuchsia-400/12 text-fuchsia-100",
    text: "text-fuchsia-300",
    wash: "from-fuchsia-500/30",
  },
  pink: {
    dot: "bg-pink-400",
    chip: "border-pink-300/30 bg-pink-400/12 text-pink-100",
    text: "text-pink-300",
    wash: "from-pink-500/30",
  },
};

/** A stable colour for tags that have none, picked from the name. */
export function fallbackTagColor(name: string): TagColor {
  let hash = 0;
  for (const character of name.toLowerCase()) {
    hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  }
  return TAG_COLORS[hash % TAG_COLORS.length] ?? "violet";
}

export function tagColor(tag: { name: string; color?: TagColor | null }) {
  return tag.color && tag.color in TONES
    ? tag.color
    : fallbackTagColor(tag.name);
}

export function tagTone(tag: {
  name: string;
  color?: TagColor | null;
}): TagTone {
  return TONES[tagColor(tag)];
}

export type TagSort = "name" | "items" | "updated";

/** Orders tags for the overview; smart tags never sink below empty ones. */
export function sortTags<
  T extends { name: string; item_count: number; updated_at?: string },
>(tags: T[], sort: TagSort): T[] {
  const byName = (left: T, right: T) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" });
  return [...tags].sort((left, right) => {
    if (sort === "items") {
      return right.item_count - left.item_count || byName(left, right);
    }
    if (sort === "updated") {
      return (
        (right.updated_at ?? "").localeCompare(left.updated_at ?? "") ||
        byName(left, right)
      );
    }
    return byName(left, right);
  });
}
