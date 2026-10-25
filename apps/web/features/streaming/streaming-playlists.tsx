"use client";

import {
  STREAMING_PROVIDER_NAMES,
  formatLastSync,
  getStreamingPlaylist,
  getStreamingPlaylistCopies,
  getStreamingPlaylistCopy,
  getStreamingPlaylists,
  parseStreamingId,
  removeStreamingPlaylistCopy,
  saveStreamingPlaylistCopy,
  type StreamingPlaylist,
  type StreamingPlaylistCopy,
  type StreamingProviderId,
  type StreamingTrack,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Copy,
  ExternalLink,
  Loader2,
  Lock,
  Pause,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { toast } from "sonner";
import { usePageTitle } from "@/components/app/title-metadata";
import EntityPageState from "@/features/library/entity-page-state";
import { AlbumArtFallback } from "@/lib/images/image-url";
import { cn } from "@/lib/cn";
import { SourceBadge } from "./source-badge";
import { SandboxNotice } from "./services-settings";
import { totalDurationLabel } from "./streaming-model";
import { StreamingTrackRow } from "./streaming-track-row";
import {
  streamingErrorMessage,
  useStreamingPlayback,
  useStreamingProviders,
} from "./use-streaming";

const copiesKey = ["streaming", "playlist-copies"] as const;

export const playlistHref = (id: string, copy = false) =>
  `/streaming/playlist?id=${encodeURIComponent(id)}${copy ? "&copy=1" : ""}`;

function Cover({ src, title }: { src: string | null; title: string }) {
  return src ? (
    <Image alt={title} className="object-cover" fill sizes="220px" src={src} />
  ) : (
    <AlbumArtFallback label="Playlist cover unavailable" />
  );
}

function PlaylistTile({
  href,
  title,
  subtitle,
  cover,
  badge,
}: {
  href: string;
  title: string;
  subtitle: string;
  cover: string | null;
  badge?: string;
}) {
  return (
    <Link className="group block min-w-0" href={href}>
      <div className="relative aspect-square overflow-hidden rounded-md border border-white/5 bg-[#111]">
        <Cover src={cover} title={title} />
        {badge && (
          <span className="absolute left-2 top-2 inline-flex items-center gap-1 rounded-full bg-black/70 px-2 py-1 text-[10.5px] font-semibold text-white backdrop-blur-md">
            <Copy aria-hidden="true" className="h-3 w-3" />
            {badge}
          </span>
        )}
      </div>
      <p className="mt-2.5 truncate text-sm font-semibold text-white group-hover:underline">
        {title}
      </p>
      <p className="truncate text-xs text-zinc-500">{subtitle}</p>
    </Link>
  );
}

const trackCount = (count: number) => `${count} track${count === 1 ? "" : "s"}`;

/** The service's playlists, beside the copies already saved in Parson. */
export function StreamingPlaylistsSection({
  provider,
}: {
  provider: StreamingProviderId;
}) {
  const playlists = useQuery({
    queryKey: ["streaming", "playlists", provider],
    queryFn: () => getStreamingPlaylists(provider, { limit: 100 }),
  });
  const copies = useQuery({
    queryKey: copiesKey,
    queryFn: getStreamingPlaylistCopies,
  });
  const saved = (copies.data ?? []).filter(
    (copy) => copy.provider === provider,
  );

  if (playlists.isPending) {
    return (
      <p className="flex items-center gap-2 text-sm text-zinc-500">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading playlists…
      </p>
    );
  }
  if (playlists.isError || !playlists.data) {
    return (
      <p className="text-sm text-zinc-400">
        Playlists could not be loaded.{" "}
        <button
          className="text-white underline-offset-4 hover:underline"
          onClick={() => void playlists.refetch()}
          type="button"
        >
          Try again
        </button>
      </p>
    );
  }
  const own = playlists.data.playlists.filter((playlist) => playlist.own);
  const followed = playlists.data.playlists.filter((playlist) => !playlist.own);
  const name = STREAMING_PROVIDER_NAMES[provider];

  return (
    <div className="space-y-10">
      <p className="flex items-start gap-2 text-[13px] leading-5 text-zinc-500">
        <Lock aria-hidden="true" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        {name} playlists are read-only here; edit them in {name}. Save a copy to
        keep one in Parson as it is today.
      </p>
      {saved.length > 0 && (
        <PlaylistGrid title="Saved in Parson">
          {saved.map((copy) => (
            <PlaylistTile
              badge="Copy"
              cover={copy.cover_url}
              href={playlistHref(copy.id, true)}
              key={copy.id}
              subtitle={`${trackCount(copy.track_count)} · saved ${formatLastSync(copy.saved_at)}`}
              title={copy.title}
            />
          ))}
        </PlaylistGrid>
      )}
      {[
        { title: "Your playlists", items: own },
        { title: "Playlists you follow", items: followed },
      ].map(
        (group) =>
          group.items.length > 0 && (
            <PlaylistGrid key={group.title} title={group.title}>
              {group.items.map((playlist) => (
                <PlaylistTile
                  badge={playlist.saved_copy_at ? "Saved" : undefined}
                  cover={playlist.cover_url}
                  href={playlistHref(playlist.id)}
                  key={playlist.id}
                  subtitle={[
                    playlist.own ? null : playlist.owner,
                    trackCount(playlist.track_count),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                  title={playlist.title}
                />
              ))}
            </PlaylistGrid>
          ),
      )}
      {!playlists.data.playlists.length && !saved.length && (
        <p className="text-sm text-zinc-500">
          No playlists on this {name} account yet.
        </p>
      )}
    </div>
  );
}

function PlaylistGrid({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <h2 className="mb-4 text-lg font-semibold text-white">{title}</h2>
      <div className="grid grid-cols-2 gap-x-4 gap-y-8 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        {children}
      </div>
    </section>
  );
}

function useCopyActions(id: string) {
  const queryClient = useQueryClient();
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: ["streaming"] });
  const save = useMutation({
    mutationFn: () => saveStreamingPlaylistCopy(id),
    onSuccess: async (copy) => {
      toast.success(
        `Saved a copy of ${copy.title} with ${trackCount(copy.track_count)}`,
      );
      await refresh();
    },
    onError: (error) =>
      toast(streamingErrorMessage(error, "Could not save a copy.")),
  });
  const remove = useMutation({
    mutationFn: () => removeStreamingPlaylistCopy(id),
    onSuccess: async () => {
      toast("Copy removed. The service playlist is unchanged.");
      await refresh();
    },
    onError: (error) =>
      toast(streamingErrorMessage(error, "Could not remove the copy.")),
  });
  return { save, remove };
}

type PlaylistLike = {
  id: string;
  provider: StreamingProviderId;
  title: string;
  description: string | null;
  owner: string | null;
  own: boolean;
  cover_url: string | null;
  duration: number;
  web_url: string | null;
  tracks: StreamingTrack[];
};

/** `/streaming/playlist?id=tidal:…` and, with `&copy=1`, its saved copy. */
export default function StreamingPlaylistPage() {
  const params = useSearchParams();
  const id = params.get("id") ?? "";
  const showCopy = params.get("copy") === "1";
  const parsed = parseStreamingId(id);
  const { sandbox } = useStreamingProviders();
  const live = useQuery({
    queryKey: ["streaming", "playlist", id],
    queryFn: () => getStreamingPlaylist(id),
    enabled: Boolean(parsed) && !showCopy,
    retry: false,
  });
  const copy = useQuery({
    queryKey: ["streaming", "playlist-copy", id],
    queryFn: () => getStreamingPlaylistCopy(id),
    enabled: Boolean(parsed) && showCopy,
    retry: false,
  });
  const active = showCopy ? copy : live;
  const playlist: PlaylistLike | undefined = showCopy
    ? copy.data?.playlist
    : live.data;
  usePageTitle(playlist?.title ?? null);

  if (!parsed) return <EntityPageState kind="playlist" />;
  if (active.isPending) return <EntityPageState kind="playlist" loading />;
  if (active.isError || !playlist) {
    return (
      <div className="mx-auto max-w-xl px-6 py-20 text-center">
        <p className="text-lg font-semibold text-white">
          This {STREAMING_PROVIDER_NAMES[parsed.provider]} playlist could not be
          loaded
        </p>
        <p className="mt-2 text-sm text-zinc-400">
          {streamingErrorMessage(
            active.error,
            "Check your connection and try again.",
          )}
        </p>
        {!showCopy && (
          <Link
            className="mt-6 inline-flex rounded-full border border-white/15 px-5 py-2 text-sm font-semibold text-white"
            href={playlistHref(id, true)}
          >
            Open the saved copy
          </Link>
        )}
      </div>
    );
  }

  return (
    <PlaylistView
      copy={showCopy ? (copy.data ?? null) : null}
      live={showCopy ? null : (live.data ?? null)}
      playlist={playlist}
      sandbox={sandbox}
    />
  );
}

function PlaylistView({
  playlist,
  live,
  copy,
  sandbox,
}: {
  playlist: PlaylistLike;
  live: StreamingPlaylist | null;
  copy: StreamingPlaylistCopy | null;
  sandbox: boolean;
}) {
  const playback = useStreamingPlayback();
  const { save, remove } = useCopyActions(playlist.id);
  const playing =
    playback.isPlaying &&
    playlist.tracks.some((track) => track.id === playback.currentSongId);
  const name = STREAMING_PROVIDER_NAMES[playlist.provider];
  const savedAt = copy?.saved_at ?? live?.saved_copy_at ?? null;

  return (
    <div className="mx-auto max-w-225 px-5 pb-24 pt-8 text-zinc-50 sm:px-7">
      {sandbox && (
        <div className="mb-6">
          <SandboxNotice compact />
        </div>
      )}
      <div className="mb-8 flex flex-col items-start gap-6 sm:flex-row sm:items-end">
        <div className="relative h-44 w-44 shrink-0 overflow-hidden rounded-md border border-white/5 sm:h-52 sm:w-52">
          <Cover src={playlist.cover_url} title={playlist.title} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <SourceBadge provider={playlist.provider} />
            <span className="inline-flex items-center gap-1 rounded-[5px] border border-white/10 px-1.5 py-0.5 text-[10.5px] font-semibold text-zinc-400">
              {copy ? (
                <>
                  <Copy aria-hidden="true" className="h-3 w-3" /> Saved copy
                </>
              ) : (
                <>
                  <Lock aria-hidden="true" className="h-3 w-3" /> Read-only
                </>
              )}
            </span>
          </div>
          <h1 className="mb-3 text-balance text-4xl font-black leading-[0.98] text-white sm:text-5xl">
            {playlist.title}
          </h1>
          {playlist.description && (
            <p className="mb-3 max-w-xl text-sm leading-6 text-zinc-400">
              {playlist.description}
            </p>
          )}
          <p className="text-sm font-medium text-zinc-300">
            {[
              playlist.own ? "Your playlist" : playlist.owner,
              trackCount(playlist.tracks.length),
              totalDurationLabel(playlist.duration),
            ]
              .filter(Boolean)
              .join(" • ")}
          </p>
        </div>
      </div>

      <div className="mb-7 flex flex-wrap items-center gap-3">
        <button
          aria-label={`${playing ? "Pause" : "Play"} ${playlist.title}`}
          className="flex h-12 w-12 items-center justify-center rounded-full bg-white text-black transition-all hover:scale-105 hover:bg-zinc-100"
          onClick={() =>
            playing
              ? playback.playTracks(
                  playlist.tracks,
                  Math.max(
                    0,
                    playlist.tracks.findIndex(
                      (track) => track.id === playback.currentSongId,
                    ),
                  ),
                )
              : playback.playTracks(playlist.tracks)
          }
          type="button"
        >
          {playing ? (
            <Pause className="h-6 w-6 fill-black text-black" />
          ) : (
            <Play className="ml-1 h-6 w-6 fill-black text-black" />
          )}
        </button>
        {live && (
          <button
            className="inline-flex h-10 items-center gap-2 rounded-full border border-white/15 px-4 text-sm font-semibold text-white hover:bg-white/6 disabled:opacity-60"
            disabled={save.isPending}
            onClick={() => save.mutate()}
            type="button"
          >
            {save.isPending ? (
              <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
            ) : savedAt ? (
              <RefreshCw aria-hidden="true" className="h-4 w-4" />
            ) : (
              <Copy aria-hidden="true" className="h-4 w-4" />
            )}
            {savedAt ? "Update saved copy" : "Save a copy in Parson"}
          </button>
        )}
        {savedAt && (
          <Link
            className={cn(
              "text-sm text-zinc-400 underline-offset-4 hover:text-white hover:underline",
              copy && "hidden",
            )}
            href={playlistHref(playlist.id, true)}
          >
            View copy saved {formatLastSync(savedAt)}
          </Link>
        )}
        {copy && (
          <>
            <Link
              className="inline-flex h-10 items-center rounded-full border border-white/15 px-4 text-sm font-semibold text-white hover:bg-white/6"
              href={playlistHref(playlist.id)}
            >
              Open the live playlist
            </Link>
            <button
              className="inline-flex h-10 items-center gap-2 rounded-full px-3 text-sm text-zinc-400 hover:bg-white/6 hover:text-white disabled:opacity-60"
              disabled={remove.isPending}
              onClick={() => remove.mutate()}
              type="button"
            >
              <Trash2 aria-hidden="true" className="h-4 w-4" />
              Remove copy
            </button>
          </>
        )}
        {playlist.web_url && (
          <a
            className="ml-auto inline-flex items-center gap-1.5 text-sm text-zinc-500 hover:text-zinc-200"
            href={playlist.web_url}
            rel="noreferrer"
            target="_blank"
          >
            Edit in {name}
            <ExternalLink className="h-3.5 w-3.5" />
          </a>
        )}
      </div>

      <div className="flex flex-col space-y-1">
        {playlist.tracks.map((track, index) => (
          <StreamingTrackRow
            active={playback.currentSongId === track.id}
            index={index}
            key={`${track.id}-${index}`}
            onPlay={() => playback.playTracks(playlist.tracks, index)}
            playing={playback.isPlaying && playback.currentSongId === track.id}
            showAlbum
            showFormat
            track={{ ...track, track_number: index + 1 }}
          />
        ))}
      </div>

      <footer className="mt-10 text-xs text-zinc-500">
        {copy
          ? `Copy saved ${formatLastSync(copy.saved_at)}. It keeps these tracks even if the ${name} playlist changes; play them while signed in to ${name}.`
          : `Read from ${name}. Parson never changes this playlist.`}
      </footer>
    </div>
  );
}

/** Saved TIDAL and Qobuz playlist copies, among Library > Playlists. */
export function SavedPlaylistCopyCards() {
  const copies = useQuery({
    queryKey: copiesKey,
    queryFn: getStreamingPlaylistCopies,
    staleTime: 30_000,
  });
  return (
    <>
      {(copies.data ?? []).map((copy) => (
        <Link
          aria-label={`Open ${copy.title}, saved from ${STREAMING_PROVIDER_NAMES[copy.provider]}`}
          className="group flex min-w-0 items-center gap-4 overflow-hidden rounded-xl border border-white/8 bg-white/2.5 p-4 transition-colors hover:bg-white/6 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-white/30"
          href={playlistHref(copy.id, true)}
          key={copy.id}
        >
          <div className="relative h-14 w-14 shrink-0 overflow-hidden rounded-lg">
            <Cover src={copy.cover_url} title={copy.title} />
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-sm font-semibold text-zinc-100">
              {copy.title}
            </h2>
            <p className="mt-1 flex items-center gap-1.5 text-xs text-zinc-500">
              <SourceBadge provider={copy.provider} />
              <span>· copy · {trackCount(copy.track_count)}</span>
            </p>
          </div>
        </Link>
      ))}
    </>
  );
}
