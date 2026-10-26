"use client";

import {
  STREAMING_PROVIDER_NAMES,
  getStreamingLibrary,
  type StreamingProviderId,
  type StreamingTrack,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { HardDrive, Layers } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { cn } from "@/lib/cn";
import { ProviderMark } from "./source-badge";
import {
  parseAlbumSourceFilter,
  streamingAlbumEntry,
  type AlbumSourceFilter,
} from "./streaming-model";
import { StreamingTrackRow } from "./streaming-track-row";
import { useStreamingPlayback, useStreamingProviders } from "./use-streaming";

const MAX_STREAMING_LIBRARY_ITEMS = 500;

/** Source filter state (kept in `?source=`) and the streaming library data it needs. */
export function useLibrarySources(kind: "album" | "track") {
  const [source, setSourceState] = useState<AlbumSourceFilter>(() =>
    typeof window === "undefined"
      ? "all"
      : parseAlbumSourceFilter(
          new URLSearchParams(window.location.search).get("source"),
        ),
  );
  const { providers } = useStreamingProviders();
  const available = providers
    .filter(
      (provider) =>
        provider.status === "signed_in" ||
        provider.library.albums + provider.library.tracks > 0,
    )
    .map((provider) => provider.id);
  const streamingSource =
    source === "tidal" || source === "qobuz" ? source : undefined;
  const items = useQuery({
    queryKey: ["streaming", "library", kind, streamingSource ?? "all"],
    queryFn: () =>
      getStreamingLibrary({
        kind,
        provider: streamingSource,
        sort: kind === "album" ? "artist" : "recent",
        limit: MAX_STREAMING_LIBRARY_ITEMS,
      }),
    enabled: available.length > 0 && source !== "local",
  });

  const setSource = (next: AlbumSourceFilter) => {
    setSourceState(next);
    const params = new URLSearchParams(window.location.search);
    if (next === "all") params.delete("source");
    else params.set("source", next);
    window.history.replaceState(
      window.history.state,
      "",
      `/library${params.size ? `?${params}` : ""}`,
    );
  };

  return {
    source,
    setSource,
    available,
    showFilters: available.length > 0,
    loading: items.isPending && items.fetchStatus !== "idle",
    albums:
      kind === "album"
        ? (items.data?.items ?? []).map(streamingAlbumEntry)
        : [],
    tracks:
      kind === "track"
        ? (items.data?.items ?? [])
            .map((item) => item.item as StreamingTrack | null)
            .filter((track): track is StreamingTrack => Boolean(track?.id))
        : [],
    total: items.data?.total ?? 0,
  };
}

/** Roon's Focus > Storage location chips: All, Local files, TIDAL/Qobuz library. */
export function SourceFilterChips({
  source,
  available,
  onChange,
}: {
  source: AlbumSourceFilter;
  available: StreamingProviderId[];
  onChange: (source: AlbumSourceFilter) => void;
}) {
  const chips: {
    value: AlbumSourceFilter;
    label: string;
    icon: React.ReactNode;
  }[] = [
    {
      value: "all",
      label: "All sources",
      icon: <Layers className="h-3.5 w-3.5" />,
    },
    {
      value: "local",
      label: "Local files",
      icon: <HardDrive className="h-3.5 w-3.5" />,
    },
    ...available.map((provider) => ({
      value: provider,
      label: `${STREAMING_PROVIDER_NAMES[provider]} library`,
      icon: (
        <ProviderMark
          className="h-3.5 w-3.5 text-[8px]"
          onLight={source === provider}
          provider={provider}
        />
      ),
    })),
  ];
  return (
    <div
      aria-label="Filter by source"
      className="flex flex-wrap gap-2"
      role="radiogroup"
    >
      {chips.map((chip) => (
        <button
          aria-checked={source === chip.value}
          className={cn(
            "inline-flex h-8 items-center gap-2 rounded-full border px-3 text-[13px] font-medium transition-colors",
            source === chip.value
              ? "border-white bg-white text-black"
              : "border-white/12 text-zinc-300 hover:border-white/30 hover:text-white",
          )}
          key={chip.value}
          onClick={() => onChange(chip.value)}
          role="radio"
          type="button"
        >
          {chip.icon}
          {chip.label}
        </button>
      ))}
    </div>
  );
}

export function StreamingLibraryEmpty({
  provider,
  kind,
}: {
  provider: StreamingProviderId;
  kind: "albums" | "tracks";
}) {
  const name = STREAMING_PROVIDER_NAMES[provider];
  return (
    <div className="rounded-xl border border-white/8 px-6 py-12 text-center">
      <p className="font-semibold text-zinc-100">
        No {name} {kind} in your library yet
      </p>
      <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-zinc-400">
        Search {name} and choose Add to library, or import your {name} favorites
        with Sync library now in{" "}
        <Link
          className="text-white underline-offset-4 hover:underline"
          href="/settings?tab=services"
        >
          Settings, Services
        </Link>
        .
      </p>
    </div>
  );
}

export function StreamingTrackList({
  tracks,
  title,
}: {
  tracks: StreamingTrack[];
  title?: string;
}) {
  const playback = useStreamingPlayback();
  if (!tracks.length) return null;
  return (
    <section className="space-y-2">
      {title && <h2 className="text-lg font-semibold text-white">{title}</h2>}
      <div className="space-y-0.5">
        {tracks.map((track, index) => (
          <StreamingTrackRow
            active={playback.currentSongId === track.id}
            index={index}
            key={track.id}
            onPlay={() => playback.playTracks(tracks, index)}
            playing={playback.isPlaying && playback.currentSongId === track.id}
            showAlbum
            showFormat
            track={{ ...track, track_number: index + 1, in_library: true }}
          />
        ))}
      </div>
    </section>
  );
}
