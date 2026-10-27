"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import AlbumCard from "@/features/library/album-card";
import CompactTrackList from "@/features/library/compact-track-list";
import { PersonAvatar } from "@/features/library/credits/person-avatar";
import { TAGS_KEY } from "@/features/library/focus/tag-submenu";
import { usePlayFocus } from "@/features/library/focus/use-play-focus";
import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  deleteTag,
  encodeFocus,
  getTagPage,
  tagCountsSummary,
  untagItems,
  type FocusQuery,
  type TaggedEntity,
  type UserTag,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ListMusic,
  Loader2,
  Music2,
  Pencil,
  Play,
  ScrollText,
  Shuffle,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useState, type ReactNode } from "react";
import { toast } from "sonner";
import { tagTone } from "./tag-colors";
import TagEditorDialog, { SmartQuerySummary } from "./tag-editor-dialog";

export const TAG_PAGE_KEY = ["library", "tag"] as const;

/** The Library, focused on one tag (or on a smart tag's own focus). */
export function tagFocusQuery(tag: Pick<UserTag, "id" | "name">): FocusQuery {
  return {
    entity: "albums",
    filters: [{ facet: "tag", tag_id: tag.id, name: tag.name }],
  };
}

export function libraryFocusHref(query: FocusQuery): string {
  const entity = query.entity ?? "albums";
  return `/library?${new URLSearchParams({
    ...(entity === "albums" ? {} : { view: entity }),
    focus: encodeFocus(query),
  })}`;
}

function Section({
  children,
  count,
  title,
}: {
  children: ReactNode;
  count: number;
  title: string;
}) {
  if (!count) return null;
  return (
    <section className="mb-12">
      <h2 className="mb-4 flex items-baseline gap-2 text-2xl font-bold text-white">
        {title}
        <span className="text-base font-medium text-zinc-500">{count}</span>
      </h2>
      {children}
    </section>
  );
}

function RemoveButton({
  label,
  onClick,
  className,
}: {
  label: string;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      aria-label={label}
      className={cn(
        "grid h-7 w-7 place-items-center rounded-full bg-black/70 text-zinc-300 opacity-0 backdrop-blur transition-opacity hover:text-white focus-visible:opacity-100 group-hover:opacity-100",
        className,
      )}
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

export default function TagDetails() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const queryClient = useQueryClient();
  const id = Number(searchParams.get("id"));
  const valid = Number.isInteger(id) && id > 0;
  const page = useQuery({
    queryKey: [...TAG_PAGE_KEY, id],
    queryFn: () => getTagPage(id),
    enabled: valid,
  });
  const playFocus = usePlayFocus();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: TAG_PAGE_KEY });
    await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
    await queryClient.invalidateQueries({ queryKey: ["library", "focus"] });
  };
  const untag = useMutation({
    mutationFn: ({ type, ids }: { type: TaggedEntity; ids: string[] }) =>
      untagItems(id, type, ids),
    onSuccess: refresh,
    onError: () => toast("Could not update the tag."),
  });
  const remove = useMutation({
    mutationFn: () => deleteTag(id),
    onSuccess: async () => {
      toast.success(`Deleted #${page.data?.tag.name ?? "tag"}`);
      await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
      router.push("/tags");
    },
    onError: () => toast("Could not delete the tag."),
  });

  if (!valid || page.isError) {
    return (
      <div className="px-8 pt-24 text-center text-sm text-zinc-400">
        This tag could not be found.{" "}
        <Link className="text-white underline" href="/tags">
          All tags
        </Link>
      </div>
    );
  }
  if (page.isPending) {
    return (
      <div className="flex justify-center pt-32 text-zinc-500">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }

  const { tag, albums, artists, songs, works, playlists } = page.data;
  const tone = tagTone(tag);
  const smart = tag.kind === "smart";
  const focusQuery = smart && tag.query ? tag.query : tagFocusQuery(tag);
  const playQuery: FocusQuery = { ...tagFocusQuery(tag), entity: "songs" };
  const heroCover = albums[0]?.coverPath
    ? getLibraryImageUrl(albums[0].coverPath, getBaseURL)
    : null;
  const empty =
    !albums.length &&
    !artists.length &&
    !songs.length &&
    !works.length &&
    !playlists.length;
  const removable = !smart;

  return (
    <div className="relative text-zinc-50">
      <div className="pointer-events-none absolute inset-x-0 top-0 h-[460px] overflow-hidden [mask-image:linear-gradient(to_bottom,black_45%,transparent)]">
        {heroCover && (
          <Image
            alt=""
            className="scale-125 object-cover opacity-30 blur-3xl"
            fill
            priority
            sizes="100vw"
            src={heroCover}
          />
        )}
        <div
          className={cn(
            "absolute inset-0 bg-linear-to-b to-transparent",
            tone.wash,
          )}
        />
      </div>
      <div className="relative mx-auto max-w-6xl px-5 pb-24 pt-14 sm:px-8">
        <header className="mb-12">
          <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-400">
            {smart ? (
              <>
                <Sparkles className="h-3.5 w-3.5 text-violet-300" /> Smart tag
              </>
            ) : (
              <>
                <span className={cn("h-2 w-2 rounded-full", tone.dot)} /> Tag
              </>
            )}
          </p>
          <h1 className="break-words text-5xl font-black leading-none text-white sm:text-7xl">
            <span className={tone.text}>#</span>
            {tag.name}
          </h1>
          {tag.description && (
            <p className="mt-4 max-w-2xl text-base leading-7 text-zinc-300">
              {tag.description}
            </p>
          )}
          <p className="mt-3 text-sm text-zinc-400">{tagCountsSummary(tag)}</p>
          {smart && tag.query && (
            <div className="mt-4 max-w-3xl rounded-xl border border-white/8 bg-black/30 px-3.5 py-2.5 backdrop-blur">
              <SmartQuerySummary query={tag.query} />
            </div>
          )}
          <div className="mt-7 flex flex-wrap items-center gap-3">
            <button
              aria-label={`Play #${tag.name}`}
              className="grid h-12 w-12 place-items-center rounded-full bg-white text-black transition-transform hover:scale-105 disabled:opacity-50"
              disabled={empty}
              onClick={() => void playFocus(playQuery, false)}
              type="button"
            >
              <Play className="ml-0.5 h-6 w-6 fill-black" />
            </button>
            <button
              aria-label={`Shuffle #${tag.name}`}
              className="grid h-10 w-10 place-items-center rounded-full border border-white/15 text-zinc-200 hover:border-white/35 hover:text-white disabled:opacity-50"
              disabled={empty}
              onClick={() => void playFocus(playQuery, true)}
              type="button"
            >
              <Shuffle className="h-[18px] w-[18px]" />
            </button>
            <Button asChild className="rounded-full" variant="secondary">
              <Link href={libraryFocusHref(focusQuery)}>
                <SlidersHorizontal />
                {smart ? "Open focus in Library" : "Focus Library on this tag"}
              </Link>
            </Button>
            <button
              aria-label="Edit tag"
              className="grid h-10 w-10 place-items-center rounded-full text-zinc-400 hover:bg-white/8 hover:text-white"
              onClick={() => setEditing(true)}
              title="Edit"
              type="button"
            >
              <Pencil className="h-4 w-4" />
            </button>
            <button
              aria-label="Delete tag"
              className="grid h-10 w-10 place-items-center rounded-full text-zinc-400 hover:bg-red-500/15 hover:text-red-300"
              onClick={() => setDeleting(true)}
              title="Delete"
              type="button"
            >
              <Trash2 className="h-4 w-4" />
            </button>
          </div>
        </header>

        {empty && (
          <div className="rounded-2xl border border-dashed border-white/10 px-8 py-14 text-center">
            <p className="text-base font-semibold text-white">
              {smart ? "Nothing matches this focus yet" : "Nothing tagged yet"}
            </p>
            <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-zinc-400">
              {smart
                ? "As music that fits arrives, it appears here by itself."
                : `Use “Tags” in the menu of any album, artist or track, or Manage tags on a page, to add music to #${tag.name}.`}
            </p>
          </div>
        )}

        <Section count={albums.length} title="Albums">
          <ul className="grid grid-cols-2 gap-x-4 gap-y-7 sm:grid-cols-3 lg:grid-cols-5">
            {albums.map((album) => (
              <li className="group relative min-w-0" key={album.id}>
                <AlbumCard
                  album_cover={album.coverPath}
                  album_id={album.id}
                  album_name={album.name}
                  artist_id={album.artistId}
                  artist_name={album.artistName}
                  first_release_date={album.releaseYear}
                  formatLabel={
                    album.quality === "hires"
                      ? "Hi-Res"
                      : album.quality === "cd"
                        ? "Lossless"
                        : null
                  }
                />
                {removable && (
                  <RemoveButton
                    className="absolute right-2 top-2 z-10"
                    label={`Remove ${album.name} from #${tag.name}`}
                    onClick={() =>
                      untag.mutate({ type: "album", ids: [album.id] })
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>

        <Section count={artists.length} title="Artists">
          <ul className="grid grid-cols-3 gap-x-4 gap-y-6 sm:grid-cols-4 lg:grid-cols-6">
            {artists.map((artist) => (
              <li className="group relative min-w-0" key={artist.id}>
                <Link
                  className="flex flex-col items-center text-center"
                  href={`/artist?id=${encodeURIComponent(artist.id)}`}
                >
                  <PersonAvatar
                    className="transition-transform duration-300 group-hover:scale-[1.03]"
                    imageUrl={
                      artist.artworkPath
                        ? getLibraryImageUrl(artist.artworkPath, getBaseURL)
                        : null
                    }
                    name={artist.name}
                    size={112}
                  />
                  <span className="mt-2 line-clamp-2 text-sm font-semibold text-white group-hover:underline">
                    {artist.name}
                  </span>
                  <span className="text-xs text-zinc-500">
                    {artist.albumCount}{" "}
                    {artist.albumCount === 1 ? "album" : "albums"}
                  </span>
                </Link>
                {removable && (
                  <RemoveButton
                    className="absolute right-3 top-0 z-10"
                    label={`Remove ${artist.name} from #${tag.name}`}
                    onClick={() =>
                      untag.mutate({ type: "artist", ids: [artist.id] })
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>

        <Section count={songs.length} title="Tracks">
          <CompactTrackList
            songs={songs}
            trailing={(song) =>
              removable ? (
                <RemoveButton
                  className="bg-transparent"
                  label={`Remove ${song.name} from #${tag.name}`}
                  onClick={() =>
                    untag.mutate({ type: "track", ids: [song.id] })
                  }
                />
              ) : null
            }
          />
        </Section>

        <Section count={works.length} title="Works">
          <ul className="grid gap-2 sm:grid-cols-2">
            {works.map((work) => (
              <li
                className="group flex items-center gap-3 rounded-xl border border-white/8 bg-white/[0.02] px-4 py-3"
                key={work.id}
              >
                <ScrollText className="h-5 w-5 shrink-0 text-zinc-500" />
                <Link
                  className="min-w-0 flex-1"
                  href={`/work?id=${encodeURIComponent(work.id)}`}
                >
                  <p className="truncate text-sm font-semibold text-white group-hover:underline">
                    {work.title}
                  </p>
                  <p className="truncate text-xs text-zinc-500">
                    {[
                      work.composers
                        .map((composer) => composer.name)
                        .join(", "),
                      work.form,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </p>
                </Link>
                {removable && (
                  <RemoveButton
                    className="bg-transparent"
                    label={`Remove ${work.title} from #${tag.name}`}
                    onClick={() =>
                      untag.mutate({ type: "work", ids: [work.id] })
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>

        <Section count={playlists.length} title="Playlists">
          <ul className="grid gap-2 sm:grid-cols-2">
            {playlists.map((playlist) => (
              <li
                className="group flex items-center gap-3 rounded-xl border border-white/8 bg-white/[0.02] px-4 py-3"
                key={playlist.id}
              >
                <ListMusic className="h-5 w-5 shrink-0 text-zinc-500" />
                <Link
                  className="min-w-0 flex-1"
                  href={`/playlist?id=${playlist.id}`}
                >
                  <p className="truncate text-sm font-semibold text-white group-hover:underline">
                    {playlist.name}
                  </p>
                  <p className="flex items-center gap-1 text-xs text-zinc-500">
                    <Music2 className="h-3 w-3" /> {playlist.songCount}{" "}
                    {playlist.songCount === 1 ? "song" : "songs"}
                  </p>
                </Link>
                {removable && (
                  <RemoveButton
                    className="bg-transparent"
                    label={`Remove ${playlist.name} from #${tag.name}`}
                    onClick={() =>
                      untag.mutate({
                        type: "playlist",
                        ids: [String(playlist.id)],
                      })
                    }
                  />
                )}
              </li>
            ))}
          </ul>
        </Section>
      </div>

      {editing && (
        <TagEditorDialog
          onOpenChange={setEditing}
          onSaved={() => void refresh()}
          open={editing}
          tag={tag}
        />
      )}
      <Dialog onOpenChange={setDeleting} open={deleting}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Delete #{tag.name}?</DialogTitle>
            <DialogDescription>
              {smart
                ? "The music stays in your library. Only the smart tag goes."
                : "The music stays in your library; it just won’t carry this tag any more."}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button onClick={() => setDeleting(false)} variant="ghost">
              Cancel
            </Button>
            <Button
              disabled={remove.isPending}
              onClick={() => remove.mutate()}
              variant="destructive"
            >
              {remove.isPending && <Loader2 className="animate-spin" />}
              Delete tag
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
