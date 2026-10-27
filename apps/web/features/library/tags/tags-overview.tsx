"use client";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  getTagPreviews,
  getTags,
  tagCountsSummary,
  type UserTag,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowDownUp,
  Hash,
  Loader2,
  Plus,
  Search,
  Sparkles,
} from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useMemo, useState } from "react";
import { TAGS_KEY } from "@/features/library/focus/tag-submenu";
import { sortTags, tagTone, type TagSort } from "./tag-colors";
import TagEditorDialog from "./tag-editor-dialog";

type Kind = "all" | "manual" | "smart";

const SORTS: { value: TagSort; label: string }[] = [
  { value: "name", label: "Name" },
  { value: "items", label: "Most items" },
  { value: "updated", label: "Recently changed" },
];

function CoverMosaic({ covers, tag }: { covers: string[]; tag: UserTag }) {
  const tone = tagTone(tag);
  const urls = covers
    .map((cover) => getLibraryImageUrl(cover, getBaseURL))
    .filter((url): url is string => Boolean(url));
  if (!urls.length) {
    return (
      <div
        className={cn(
          "absolute inset-0 grid place-items-center bg-linear-to-br to-zinc-950",
          tone.wash,
        )}
      >
        <Hash
          className={cn("h-14 w-14 opacity-70", tone.text)}
          strokeWidth={1.5}
        />
      </div>
    );
  }
  const tiles = urls.length >= 4 ? urls.slice(0, 4) : urls.slice(0, 1);
  return (
    <div
      className={cn(
        "absolute inset-0 grid",
        tiles.length === 4 ? "grid-cols-2 grid-rows-2" : "grid-cols-1",
      )}
    >
      {tiles.map((url, index) => (
        <div className="relative overflow-hidden" key={`${url}-${index}`}>
          <Image
            alt=""
            className="object-cover transition-transform duration-500 group-hover:scale-[1.04]"
            fill
            sizes="(min-width: 1024px) 120px, 25vw"
            src={url}
          />
        </div>
      ))}
    </div>
  );
}

function TagTile({ covers, tag }: { covers: string[]; tag: UserTag }) {
  const tone = tagTone(tag);
  const smart = tag.kind === "smart";
  return (
    <li className="min-w-0">
      <Link
        className="group block focus-visible:outline-none"
        href={`/tag?id=${tag.id}`}
      >
        <div className="relative aspect-square overflow-hidden rounded-xl border border-white/8 bg-zinc-900 shadow-[0_20px_40px_-24px_rgba(0,0,0,0.9)] transition-colors group-hover:border-white/20 group-focus-visible:ring-2 group-focus-visible:ring-white/50">
          <CoverMosaic covers={covers} tag={tag} />
          <div className="absolute inset-0 bg-linear-to-t from-black/85 via-black/20 to-transparent" />
          <div
            className={cn("absolute inset-x-0 top-0 h-1 opacity-90", tone.dot)}
          />
          {smart && (
            <span className="absolute right-2.5 top-3 flex items-center gap-1 rounded-full border border-white/15 bg-black/55 px-2 py-0.5 text-[11px] font-semibold text-mujin-copper-hover backdrop-blur">
              <Sparkles className="h-3 w-3" /> Smart
            </span>
          )}
          <div className="absolute inset-x-0 bottom-0 p-3.5">
            <p className="flex items-center gap-1.5 truncate text-lg font-bold leading-6 text-white">
              <span className={cn("font-black", tone.text)}>#</span>
              <span className="truncate">{tag.name}</span>
            </p>
            <p className="truncate text-xs text-zinc-300/80">
              {tagCountsSummary(tag)}
            </p>
          </div>
        </div>
        {tag.description && (
          <p className="mt-2 line-clamp-2 text-xs leading-5 text-zinc-500">
            {tag.description}
          </p>
        )}
      </Link>
    </li>
  );
}

export default function TagsOverview() {
  const tags = useQuery({ queryKey: TAGS_KEY, queryFn: getTags });
  const previews = useQuery({
    queryKey: [...TAGS_KEY, "previews"],
    queryFn: getTagPreviews,
    enabled: Boolean(tags.data?.length),
  });
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<TagSort>("name");
  const [kind, setKind] = useState<Kind>("all");
  const [creating, setCreating] = useState(false);

  const all = tags.data ?? [];
  const smartCount = all.filter((tag) => tag.kind === "smart").length;
  const visible = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return sortTags(
      all.filter(
        (tag) =>
          (kind === "all" || (kind === "smart") === (tag.kind === "smart")) &&
          (!needle ||
            tag.name.toLowerCase().includes(needle) ||
            tag.description?.toLowerCase().includes(needle)),
      ),
      sort,
    );
  }, [all, kind, search, sort]);

  return (
    <div className="relative text-zinc-50">
      <div className="mx-auto max-w-6xl px-5 pb-24 pt-10 sm:px-8">
        <header className="mb-8 flex flex-wrap items-end justify-between gap-5">
          <div>
            <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
              <Hash className="h-3.5 w-3.5" /> Browse
            </p>
            <h1 className="text-4xl font-black text-white sm:text-5xl">Tags</h1>
            <p className="mt-3 text-sm text-zinc-400">
              {tags.isPending
                ? "Loading…"
                : `${all.length} ${all.length === 1 ? "tag" : "tags"}${
                    smartCount ? ` · ${smartCount} smart` : ""
                  }`}
            </p>
          </div>
          <div className="flex gap-2">
            <Button asChild variant="ghost">
              <Link href="/library">
                <Sparkles className="text-mujin-copper" /> Smart tag from a
                focus
              </Link>
            </Button>
            <Button className="rounded-full" onClick={() => setCreating(true)}>
              <Plus /> New tag
            </Button>
          </div>
        </header>

        {all.length > 0 && (
          <div className="mb-6 flex flex-wrap items-center gap-2">
            <div
              aria-label="Kind"
              className="flex rounded-full border border-white/10 p-0.5"
              role="radiogroup"
            >
              {(["all", "manual", "smart"] as Kind[]).map((option) => (
                <button
                  aria-checked={kind === option}
                  className={cn(
                    "h-8 rounded-full px-3.5 text-sm font-medium capitalize transition-colors",
                    kind === option
                      ? "bg-white text-black"
                      : "text-zinc-400 hover:text-white",
                  )}
                  key={option}
                  onClick={() => setKind(option)}
                  role="radio"
                  type="button"
                >
                  {option}
                </button>
              ))}
            </div>
            <label className="ml-auto flex h-9 w-full items-center gap-2 rounded-full border border-white/10 px-3.5 text-sm focus-within:border-white/30 sm:w-64">
              <Search className="h-4 w-4 text-zinc-500" />
              <input
                aria-label="Filter tags"
                className="min-w-0 flex-1 bg-transparent text-white outline-none placeholder:text-zinc-500"
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Filter tags"
                value={search}
              />
            </label>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  className="flex h-9 items-center gap-1.5 rounded-full px-3 text-sm text-zinc-300 hover:bg-white/6 hover:text-white"
                  type="button"
                >
                  <ArrowDownUp className="h-4 w-4" />
                  {SORTS.find((option) => option.value === sort)?.label}
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuRadioGroup
                  onValueChange={(value) => setSort(value as TagSort)}
                  value={sort}
                >
                  {SORTS.map((option) => (
                    <DropdownMenuRadioItem
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )}

        {tags.isPending ? (
          <div className="flex justify-center pt-16 text-zinc-500">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        ) : tags.isError ? (
          <p className="pt-10 text-center text-sm text-zinc-400">
            Could not load your tags.
          </p>
        ) : all.length === 0 ? (
          <div className="mx-auto mt-10 max-w-md rounded-2xl border border-white/8 bg-white/2 px-8 py-12 text-center">
            <div className="mx-auto mb-5 grid h-16 w-16 place-items-center rounded-full bg-mujin-copper/15 text-mujin-copper-hover">
              <Hash className="h-8 w-8" />
            </div>
            <h2 className="text-xl font-bold text-white">No tags yet</h2>
            <p className="mt-2 text-sm leading-6 text-zinc-400">
              Tag albums, artists, tracks, works and playlists from any menu —
              or save a Library focus as a smart tag that keeps itself up to
              date.
            </p>
            <Button
              className="mt-6 rounded-full"
              onClick={() => setCreating(true)}
            >
              <Plus /> Create a tag
            </Button>
          </div>
        ) : visible.length === 0 ? (
          <p className="pt-10 text-center text-sm text-zinc-500">
            No tags match.
          </p>
        ) : (
          <ul className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
            {visible.map((tag) => (
              <TagTile
                covers={previews.data?.get(tag.id) ?? []}
                key={tag.id}
                tag={tag}
              />
            ))}
          </ul>
        )}
      </div>
      {creating && (
        <TagEditorDialog onOpenChange={setCreating} open={creating} />
      )}
    </div>
  );
}
