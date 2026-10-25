"use client";

import getBaseURL from "@/lib/api/server-url";
import { defaultCover } from "@/lib/images/default-cover";
import {
  clearRadioDislike,
  filterRadioBans,
  getRadioDislikes,
  radioBanKindLabel,
  type RadioBan,
  type RadioBanFilter,
  type RadioDislike,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban, Disc3, Music2, ThumbsDown, Undo2, UserRound } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";
import { LibraryLoading, LibraryMessage } from "./library-view-primitives";
import { useRadioBans } from "./radio-bans";

type BannedFilter = RadioBanFilter | "disliked";

const filters: { id: BannedFilter; label: string }[] = [
  { id: "all", label: "All bans" },
  { id: "artist", label: "Artists" },
  { id: "album", label: "Albums" },
  { id: "song", label: "Tracks" },
  { id: "disliked", label: "Thumbs down" },
];

export const radioDislikesKey = ["radio-dislikes"] as const;

function imageSource(path?: string | null) {
  if (!path) return defaultCover;
  if (/^(https?:|data:)/.test(path)) return path;
  return `${getBaseURL()}/media/images/${encodeURIComponent(path)}`;
}

function banHref(ban: RadioBan): string | null {
  if (!ban.name) return null;
  if (ban.kind === "artist") return `/artist?id=${encodeURIComponent(ban.id)}`;
  if (ban.kind === "album") return `/album?id=${encodeURIComponent(ban.id)}`;
  return ban.album_id ? `/album?id=${encodeURIComponent(ban.album_id)}` : null;
}

const kindIcon = {
  artist: UserRound,
  album: Disc3,
  song: Music2,
} as const;

/**
 * Library > Banned: everything Radio, Shuffle and recommendations skip, with
 * one-click unban, plus tracks kept out by a thumbs down (each undoable).
 */
export default function BannedView() {
  const [filter, setFilter] = useState<BannedFilter>("all");
  const bans = useRadioBans();
  const counts: Record<BannedFilter, number> = {
    all: bans.bans.length,
    artist: filterRadioBans(bans.bans, "artist").length,
    album: filterRadioBans(bans.bans, "album").length,
    song: filterRadioBans(bans.bans, "song").length,
    disliked: 0,
  };
  const dislikes = useQuery({
    queryKey: radioDislikesKey,
    queryFn: getRadioDislikes,
    staleTime: 30_000,
  });
  counts.disliked = dislikes.data?.length ?? 0;

  return (
    <div className="space-y-5">
      <p className="max-w-2xl text-sm leading-6 text-zinc-400">
        Banned artists, albums and tracks never play in Radio, Shuffle or
        recommendations, starting with the next pick. You can still play them
        yourself. Ban anything from its menu, or tap a song’s heart until it
        shows the ban icon.
      </p>
      <div
        aria-label="Filter banned items"
        className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-1 [scrollbar-width:none]"
        role="radiogroup"
      >
        {filters.map((item) => (
          <button
            aria-checked={filter === item.id}
            className={`shrink-0 rounded-full border px-3 py-1.5 text-xs font-medium transition-colors ${
              filter === item.id
                ? "border-white bg-white text-black"
                : "border-white/10 text-zinc-300 hover:border-white/30 hover:text-white"
            }`}
            key={item.id}
            onClick={() => setFilter(item.id)}
            role="radio"
            type="button"
          >
            {item.label}
            <span className="ml-1.5 tabular-nums opacity-60">
              {counts[item.id]}
            </span>
          </button>
        ))}
      </div>
      {filter === "disliked" ? (
        <DislikedList
          items={dislikes.data}
          loading={dislikes.isPending}
          error={Boolean(dislikes.error)}
        />
      ) : (
        <BanList
          bans={filterRadioBans(bans.bans, filter)}
          error={Boolean(bans.error)}
          filter={filter}
          loading={bans.loading}
          onUnban={(ban) =>
            bans.setBanned(
              { kind: ban.kind, id: ban.id, name: ban.name },
              false,
            )
          }
        />
      )}
    </div>
  );
}

function BanList({
  bans,
  error,
  filter,
  loading,
  onUnban,
}: {
  bans: RadioBan[];
  error: boolean;
  filter: RadioBanFilter;
  loading: boolean;
  onUnban: (ban: RadioBan) => void;
}) {
  if (loading) return <LibraryLoading compact />;
  if (error) {
    return (
      <LibraryMessage
        title="Bans could not be loaded"
        body="Check your connection to the server and try again."
      />
    );
  }
  if (!bans.length) {
    return (
      <LibraryMessage
        title={
          filter === "all"
            ? "Nothing is banned"
            : `No banned ${radioBanKindLabel(filter)}s`
        }
        body="Right-click an artist, album or song and choose Ban to keep it out of Radio and Shuffle for good."
      />
    );
  }
  return (
    <ul className="overflow-hidden rounded-xl border border-white/8">
      {bans.map((ban) => {
        const Icon = kindIcon[ban.kind];
        const href = banHref(ban);
        const title = ban.name ?? "No longer in your library";
        return (
          <li
            className="flex min-w-0 items-center gap-3 border-b border-white/6 px-3 py-2.5 last:border-0 sm:px-4"
            key={`${ban.kind}:${ban.id}`}
          >
            <div
              className={`relative h-11 w-11 shrink-0 overflow-hidden bg-zinc-900 ${
                ban.kind === "artist" ? "rounded-full" : "rounded-md"
              }`}
            >
              {ban.image_url ? (
                <Image
                  alt=""
                  className="object-cover opacity-70"
                  fill
                  sizes="44px"
                  src={imageSource(ban.image_url)}
                  unoptimized
                />
              ) : (
                <Icon className="absolute inset-0 m-auto h-5 w-5 text-zinc-600" />
              )}
              <Ban
                aria-hidden="true"
                className="absolute bottom-0.5 right-0.5 h-3.5 w-3.5 rounded-full bg-black/80 text-rose-400"
              />
            </div>
            <div className="min-w-0 flex-1">
              {href ? (
                <Link
                  className="block truncate text-sm font-medium text-zinc-100 hover:underline"
                  href={href}
                >
                  {title}
                </Link>
              ) : (
                <p className="truncate text-sm font-medium text-zinc-400">
                  {title}
                </p>
              )}
              <p className="truncate text-xs text-zinc-500">
                <span className="capitalize">
                  {radioBanKindLabel(ban.kind)}
                </span>
                {ban.subtitle && ban.kind !== "artist"
                  ? ` · ${ban.subtitle}`
                  : ""}
              </p>
            </div>
            <button
              aria-label={`Unban ${title}`}
              className="shrink-0 rounded-full border border-white/15 px-3 py-1.5 text-xs font-semibold text-zinc-100 transition-colors hover:border-white/40 hover:bg-white/6"
              onClick={() => onUnban(ban)}
              type="button"
            >
              Unban
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function DislikedList({
  error,
  items,
  loading,
}: {
  error: boolean;
  items?: RadioDislike[];
  loading: boolean;
}) {
  const queryClient = useQueryClient();
  const undo = useMutation({
    mutationFn: (songId: string) => clearRadioDislike(songId),
    onMutate: async (songId) => {
      await queryClient.cancelQueries({ queryKey: radioDislikesKey });
      const previous =
        queryClient.getQueryData<RadioDislike[]>(radioDislikesKey);
      queryClient.setQueryData<RadioDislike[]>(radioDislikesKey, (current) =>
        (current ?? []).filter((item) => item.song.id !== songId),
      );
      return { previous };
    },
    onSuccess: () =>
      toast.success("Thumbs down undone. Radio can pick it again."),
    onError: (_error, _songId, context) => {
      queryClient.setQueryData(radioDislikesKey, context?.previous);
      toast("Could not undo this thumbs down.");
    },
    onSettled: () =>
      void queryClient.invalidateQueries({ queryKey: radioDislikesKey }),
  });
  if (loading) return <LibraryLoading compact />;
  if (error) {
    return (
      <LibraryMessage
        title="Thumbs down could not be loaded"
        body="Check your connection to the server and try again."
      />
    );
  }
  if (!items?.length) {
    return (
      <LibraryMessage
        title="No thumbs down"
        body="Tracks you thumb down in Radio stay out of it. They show up here so you can change your mind."
      />
    );
  }
  return (
    <ul className="overflow-hidden rounded-xl border border-white/8">
      {items.map(({ song }) => (
        <li
          className="flex min-w-0 items-center gap-3 border-b border-white/6 px-3 py-2.5 last:border-0 sm:px-4"
          key={song.id}
        >
          <div className="relative h-11 w-11 shrink-0 overflow-hidden rounded-md bg-zinc-900">
            <Image
              alt=""
              className="object-cover opacity-70"
              fill
              sizes="44px"
              src={imageSource(song.album_object?.cover_url)}
              unoptimized
            />
            <ThumbsDown
              aria-hidden="true"
              className="absolute bottom-0.5 right-0.5 h-3.5 w-3.5 rounded-full bg-black/80 p-px text-zinc-200"
            />
          </div>
          <div className="min-w-0 flex-1">
            <Link
              className="block truncate text-sm font-medium text-zinc-100 hover:underline"
              href={`/album?id=${encodeURIComponent(song.album_object.id)}`}
            >
              {song.name}
            </Link>
            <p className="truncate text-xs text-zinc-500">
              {song.artist} · {song.album_object.name}
            </p>
          </div>
          <button
            aria-label={`Undo thumbs down on ${song.name}`}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-white/15 px-3 py-1.5 text-xs font-semibold text-zinc-100 transition-colors hover:border-white/40 hover:bg-white/6 disabled:opacity-50"
            disabled={undo.isPending && undo.variables === song.id}
            onClick={() => undo.mutate(song.id)}
            type="button"
          >
            <Undo2 aria-hidden="true" className="h-3.5 w-3.5" />
            Undo
          </button>
        </li>
      ))}
    </ul>
  );
}
