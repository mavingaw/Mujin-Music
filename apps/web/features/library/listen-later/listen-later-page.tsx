"use client";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import AlbumCard from "@/features/library/album-card";
import CompactTrackList from "@/features/library/compact-track-list";
import { PersonAvatar } from "@/features/library/credits/person-avatar";
import {
  catalogSongsToQueue,
  shuffled,
} from "@/features/library/focus/focus-state";
import ManageTagsDialog from "@/features/library/tags/manage-tags-dialog";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import type { QueueInput } from "@/features/player/player-model";
import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  LISTEN_LATER_SORTS,
  addedAgo,
  getAlbumInfo,
  getListenLater,
  setListenLaterRemoveWhenPlayed,
  type LibraryAlbum,
  type ListenLaterEntity,
  type ListenLaterItem,
  type ListenLaterSort,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDownUp,
  Clock,
  Loader2,
  Play,
  Search,
  Shuffle,
  Tag,
  X,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useMemo, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { LISTEN_LATER_KEY, useChangeListenLater } from "./use-listen-later";
import { listenLaterSummary } from "./listen-later-model";

type Filter = "all" | ListenLaterEntity;

const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "album", label: "Albums" },
  { value: "track", label: "Tracks" },
  { value: "artist", label: "Artists" },
];

const MAX_ALBUMS_TO_PLAY = 40;

function Switch({
  checked,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={cn(
        "relative h-6 w-10 shrink-0 rounded-full transition-colors disabled:opacity-50",
        checked ? "bg-mujin-copper" : "bg-zinc-700",
      )}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      role="switch"
      type="button"
    >
      <span
        className={cn(
          "absolute left-0 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-[18px]" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

function RemoveOverlay({
  onClick,
  label,
}: {
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      aria-label={label}
      className="absolute right-2 top-2 z-10 grid h-7 w-7 place-items-center rounded-full bg-black/70 text-zinc-200 opacity-0 backdrop-blur transition-opacity hover:text-white focus-visible:opacity-100 group-hover:opacity-100"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClick();
      }}
      title={label}
      type="button"
    >
      <X className="h-3.5 w-3.5" />
    </button>
  );
}

function Section({
  action,
  title,
  count,
  children,
  show,
}: {
  action?: ReactNode;
  title: string;
  count: number;
  children: ReactNode;
  show: boolean;
}) {
  if (!show || !count) return null;
  return (
    <section className="mb-12">
      <div className="mb-4 flex items-center justify-between gap-3">
        <h2 className="flex items-baseline gap-2 text-2xl font-bold text-white">
          {title}
          <span className="text-base font-medium text-zinc-500">{count}</span>
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}

export default function ListenLaterPage() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const queryClient = useQueryClient();
  const playback = useZonePlayback();
  const change = useChangeListenLater();
  const requested = searchParams.get("type");
  const filter: Filter = FILTERS.some((option) => option.value === requested)
    ? (requested as Filter)
    : "all";
  const [sort, setSort] = useState<ListenLaterSort>("added");
  const [direction, setDirection] = useState<"asc" | "desc" | undefined>();
  const [search, setSearch] = useState("");
  const [tagging, setTagging] = useState<ListenLaterEntity | null>(null);
  const [starting, setStarting] = useState(false);

  const page = useQuery({
    queryKey: [
      ...LISTEN_LATER_KEY,
      "page",
      filter,
      sort,
      direction,
      search.trim(),
    ],
    queryFn: () =>
      getListenLater({
        type: filter,
        sort,
        direction,
        q: search,
        limit: 1000,
      }),
    placeholderData: (previous) => previous,
  });
  const setting = useMutation({
    mutationFn: setListenLaterRemoveWhenPlayed,
    onSuccess: async (enabled) => {
      toast.success(
        enabled
          ? "Played music will leave Listen Later"
          : "Listen Later keeps music after it plays",
      );
      await queryClient.invalidateQueries({ queryKey: LISTEN_LATER_KEY });
    },
    onError: () => toast("Could not change the setting."),
  });

  const items = useMemo(() => page.data?.items ?? [], [page.data]);
  const albums = items.filter((item) => item.album);
  const tracks = items.filter((item) => item.song);
  const artists = items.filter((item) => item.artist);
  const counts = page.data?.counts;
  const summary = counts ? listenLaterSummary(counts, items) : "";

  const setFilter = (next: Filter) => {
    const params = new URLSearchParams(searchParams.toString());
    if (next === "all") params.delete("type");
    else params.set("type", next);
    const query = params.toString();
    router.replace(query ? `/listen-later?${query}` : "/listen-later", {
      scroll: false,
    });
  };

  const remove = (item: ListenLaterItem, name: string) =>
    change.mutate({
      entityType: item.entityType,
      entityIds: [item.entityId],
      save: false,
      name,
    });

  const playAll = async (shuffle: boolean) => {
    setStarting(true);
    try {
      const queue: QueueInput[] = [];
      for (const item of albums.slice(0, MAX_ALBUMS_TO_PLAY)) {
        try {
          const album = (await getAlbumInfo(
            item.entityId,
            false,
          )) as LibraryAlbum;
          const artist = album.artist_object;
          if (!artist) continue;
          for (const song of album.songs ?? [])
            queue.push({ song, album, artist });
        } catch {}
      }
      queue.push(
        ...catalogSongsToQueue(
          tracks.map((item) => item.song).filter((song) => song !== undefined),
        ),
      );
      if (!queue.length) {
        toast("Nothing here to play yet.");
        return;
      }
      const ordered = shuffle ? shuffled(queue) : queue;
      await playback.playQueue(ordered);
    } finally {
      setStarting(false);
    }
  };

  const sortLabel =
    LISTEN_LATER_SORTS.find((option) => option.value === sort)?.label ?? "";
  const loading = page.isPending;
  const empty = !loading && counts?.all === 0;

  return (
    <div className="relative text-zinc-50">
      <div className="pointer-events-none absolute inset-x-0 top-0 h-80 bg-linear-to-b from-mujin-copper/12 via-mujin-copper/[0.03] to-transparent" />
      <div className="relative mx-auto max-w-6xl px-5 pb-24 pt-10 sm:px-8">
        <header className="mb-8 flex flex-wrap items-end justify-between gap-6">
          <div className="min-w-0">
            <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
              <Clock className="h-3.5 w-3.5" /> Browse
            </p>
            <h1 className="text-4xl font-black text-white sm:text-5xl">
              Listen Later
            </h1>
            <p className="mt-3 text-sm text-zinc-400">
              {loading ? "Loading…" : summary}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <label className="flex items-center gap-3 rounded-full border border-white/10 bg-white/[0.03] py-1.5 pl-4 pr-1.5 text-sm text-zinc-300">
              <span>
                Remove once played
                <span className="block text-[11px] leading-4 text-zinc-500">
                  Tracks after a play, albums after most of them
                </span>
              </span>
              <Switch
                checked={Boolean(page.data?.removeWhenPlayed)}
                disabled={setting.isPending || loading}
                label="Remove once played"
                onChange={(enabled) => setting.mutate(enabled)}
              />
            </label>
          </div>
        </header>

        {!empty && (
          <div className="mb-8 flex flex-wrap items-center gap-2">
            <button
              aria-label="Play Listen Later"
              className="grid h-12 w-12 place-items-center rounded-full bg-white text-black transition-transform hover:scale-105 disabled:opacity-60"
              disabled={starting || loading || !items.length}
              onClick={() => void playAll(false)}
              type="button"
            >
              {starting ? (
                <Loader2 className="h-5 w-5 animate-spin" />
              ) : (
                <Play className="ml-0.5 h-6 w-6 fill-black" />
              )}
            </button>
            <button
              aria-label="Shuffle Listen Later"
              className="mr-2 grid h-10 w-10 place-items-center rounded-full border border-white/15 text-zinc-200 hover:border-white/35 hover:text-white disabled:opacity-50"
              disabled={starting || loading || !items.length}
              onClick={() => void playAll(true)}
              type="button"
            >
              <Shuffle className="h-[18px] w-[18px]" />
            </button>
            <div
              aria-label="Show"
              className="flex rounded-full border border-white/10 p-0.5"
              role="radiogroup"
            >
              {FILTERS.map((option) => {
                const count =
                  option.value === "all"
                    ? counts?.all
                    : counts?.[option.value as ListenLaterEntity];
                return (
                  <button
                    aria-checked={filter === option.value}
                    className={cn(
                      "flex h-8 items-center gap-1.5 rounded-full px-3.5 text-sm font-medium transition-colors",
                      filter === option.value
                        ? "bg-white text-black"
                        : "text-zinc-400 hover:text-white",
                    )}
                    key={option.value}
                    onClick={() => setFilter(option.value)}
                    role="radio"
                    type="button"
                  >
                    {option.label}
                    {count !== undefined && option.value !== "all" && (
                      <span
                        className={cn(
                          "text-xs tabular-nums",
                          filter === option.value
                            ? "text-black/60"
                            : "text-zinc-600",
                        )}
                      >
                        {count}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
            <label className="ml-auto flex h-9 w-full items-center gap-2 rounded-full border border-white/10 px-3.5 text-sm focus-within:border-white/30 sm:w-56">
              <Search className="h-4 w-4 text-zinc-500" />
              <input
                aria-label="Filter Listen Later"
                className="min-w-0 flex-1 bg-transparent text-white outline-none placeholder:text-zinc-500"
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Filter"
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
                  {sortLabel}
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-52">
                <DropdownMenuRadioGroup
                  onValueChange={(value) => {
                    setSort(value as ListenLaterSort);
                    setDirection(undefined);
                  }}
                  value={sort}
                >
                  {LISTEN_LATER_SORTS.map((option) => (
                    <DropdownMenuRadioItem
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
                <DropdownMenuSeparator />
                <DropdownMenuRadioGroup
                  onValueChange={(value) =>
                    setDirection(value as "asc" | "desc")
                  }
                  value={direction ?? ""}
                >
                  <DropdownMenuRadioItem value="asc">
                    Ascending
                  </DropdownMenuRadioItem>
                  <DropdownMenuRadioItem value="desc">
                    Descending
                  </DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )}

        {Boolean(page.data?.unavailable) && (
          <p className="mb-6 rounded-lg border border-amber-300/20 bg-amber-300/6 px-4 py-2.5 text-sm text-amber-100">
            {page.data?.unavailable} saved{" "}
            {page.data?.unavailable === 1 ? "item is" : "items are"} no longer
            in your library and {page.data?.unavailable === 1 ? "is" : "are"}{" "}
            hidden.
          </p>
        )}

        {loading ? (
          <div className="flex justify-center pt-16 text-zinc-500">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        ) : page.isError ? (
          <p className="pt-10 text-center text-sm text-zinc-400">
            Could not load Listen Later.
          </p>
        ) : empty ? (
          <div className="mx-auto mt-6 max-w-md rounded-2xl border border-white/8 bg-white/2 px-8 py-12 text-center">
            <div className="mx-auto mb-5 grid h-16 w-16 place-items-center rounded-full bg-mujin-copper/15 text-mujin-copper-hover">
              <Clock className="h-8 w-8" />
            </div>
            <h2 className="text-xl font-bold text-white">
              Nothing saved for later
            </h2>
            <p className="mt-2 text-sm leading-6 text-zinc-400">
              Found something you want to hear properly? Choose{" "}
              <span className="text-zinc-200">Listen later</span> from any
              album, track or artist menu, or the clock button on an album page.
            </p>
            <Link
              className="mt-6 inline-flex h-10 items-center rounded-full bg-white px-5 text-sm font-semibold text-black hover:bg-zinc-200"
              href="/library"
            >
              Browse your library
            </Link>
          </div>
        ) : !items.length ? (
          <p className="pt-10 text-center text-sm text-zinc-500">
            Nothing matches.
          </p>
        ) : (
          <>
            <Section
              count={albums.length}
              show={filter === "all" || filter === "album"}
              action={<TagAllButton onTag={() => setTagging("album")} />}
              title="Albums"
            >
              <ul className="grid grid-cols-2 gap-x-4 gap-y-7 sm:grid-cols-3 lg:grid-cols-5">
                {albums.map((item) => {
                  const album = item.album!;
                  return (
                    <li className="group relative min-w-0" key={item.entityId}>
                      <AlbumCard
                        album_cover={album.coverPath}
                        album_id={album.id}
                        album_name={album.name}
                        artist_id={album.artistId}
                        artist_name={album.artistName}
                        first_release_date={album.releaseYear}
                        formatLabel={[
                          album.quality === "hires"
                            ? "Hi-Res"
                            : album.quality === "cd"
                              ? "Lossless"
                              : null,
                          addedAgo(item.addedAt),
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      />
                      <RemoveOverlay
                        label={`Remove ${album.name} from Listen Later`}
                        onClick={() => remove(item, album.name)}
                      />
                    </li>
                  );
                })}
              </ul>
            </Section>

            <Section
              count={tracks.length}
              show={filter === "all" || filter === "track"}
              action={<TagAllButton onTag={() => setTagging("track")} />}
              title="Tracks"
            >
              <CompactTrackList
                songs={tracks.map((item) => item.song!)}
                trailing={(song, index) => (
                  <>
                    <span className="hidden w-28 shrink-0 text-right text-xs text-zinc-600 md:block">
                      {addedAgo(tracks[index]?.addedAt ?? "")}
                    </span>
                    <button
                      aria-label={`Remove ${song.name} from Listen Later`}
                      className="grid h-7 w-7 place-items-center rounded-full text-zinc-500 opacity-0 transition-opacity hover:text-white focus-visible:opacity-100 group-hover:opacity-100"
                      onClick={() => remove(tracks[index]!, song.name)}
                      title="Remove"
                      type="button"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </>
                )}
              />
            </Section>

            <Section
              count={artists.length}
              show={filter === "all" || filter === "artist"}
              title="Artists"
            >
              <ul className="grid grid-cols-3 gap-x-4 gap-y-6 sm:grid-cols-4 lg:grid-cols-6">
                {artists.map((item) => {
                  const artist = item.artist!;
                  return (
                    <li className="group relative min-w-0" key={item.entityId}>
                      <Link
                        className="flex flex-col items-center text-center"
                        href={`/artist?id=${encodeURIComponent(artist.id)}`}
                      >
                        <PersonAvatar
                          className="transition-transform duration-300 group-hover:scale-[1.03]"
                          imageUrl={
                            artist.artworkPath
                              ? getLibraryImageUrl(
                                  artist.artworkPath,
                                  getBaseURL,
                                )
                              : null
                          }
                          name={artist.name}
                          size={112}
                        />
                        <span className="mt-2 line-clamp-2 text-sm font-semibold text-white group-hover:underline">
                          {artist.name}
                        </span>
                        <span className="text-xs text-zinc-500">
                          {addedAgo(item.addedAt)}
                        </span>
                      </Link>
                      <RemoveOverlay
                        label={`Remove ${artist.name} from Listen Later`}
                        onClick={() => remove(item, artist.name)}
                      />
                    </li>
                  );
                })}
              </ul>
            </Section>
          </>
        )}
      </div>
      {tagging && (
        <ManageTagsDialog
          entityIds={items
            .filter((item) => item.entityType === tagging)
            .map((item) => item.entityId)}
          entityType={tagging}
          onOpenChange={(open) => !open && setTagging(null)}
          open
          title={`Listen Later ${tagging === "album" ? "albums" : "tracks"}`}
        />
      )}
    </div>
  );
}

function TagAllButton({ onTag }: { onTag: () => void }) {
  return (
    <button
      className="flex h-8 items-center gap-1.5 rounded-full px-3 text-xs font-medium text-zinc-400 hover:bg-white/6 hover:text-white"
      onClick={onTag}
      type="button"
    >
      <Tag className="h-3.5 w-3.5" /> Tag all
    </button>
  );
}
