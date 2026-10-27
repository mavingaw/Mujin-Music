"use client";

import {
  getStreamingLibrary,
  searchStreaming,
  type StreamingAlbum,
  type StreamingLibraryItem,
  type StreamingServiceSearch,
  type StreamingTrack,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Globe, Library, Loader2, UserRound } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { cn } from "@/lib/cn";
import { AlbumArtFallback } from "@/lib/images/image-url";
import { FormatBadge, SourceBadge } from "./source-badge";
import { LibraryButton } from "./library-button";
import { formatTone, formatToneFromLabel } from "./streaming-model";
import { StreamingTrackRow } from "./streaming-track-row";
import { useStreamingPlayback, useStreamingProviders } from "./use-streaming";

export type SearchScope = "library" | "everywhere";

export function parseSearchScope(value: string | null): SearchScope {
  return value === "library" ? "library" : "everywhere";
}

/** Roon's library icon: limit results to the library, or search everywhere. */
export function SearchScopeToggle({
  scope,
  onChange,
}: {
  scope: SearchScope;
  onChange: (scope: SearchScope) => void;
}) {
  const options = [
    { value: "library", label: "In library", icon: Library },
    { value: "everywhere", label: "Everywhere", icon: Globe },
  ] as const;
  return (
    <div
      aria-label="Search scope"
      className="inline-flex rounded-full border border-white/10 bg-white/3 p-1"
      role="radiogroup"
    >
      {options.map(({ value, label, icon: Icon }) => (
        <button
          aria-checked={scope === value}
          className={cn(
            "inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-[0.8125rem] font-medium transition-colors",
            scope === value
              ? "bg-white text-black"
              : "text-zinc-400 hover:text-white",
          )}
          key={value}
          onClick={() => onChange(value)}
          role="radio"
          type="button"
        >
          <Icon aria-hidden="true" className="h-3.5 w-3.5" />
          {label}
        </button>
      ))}
    </div>
  );
}

function AlbumTile({ album }: { album: StreamingAlbum }) {
  return (
    <div className="group relative min-w-0">
      <Link
        aria-label={`View ${album.title}`}
        className="absolute inset-0 z-0 rounded-lg"
        href={`/album?id=${encodeURIComponent(album.id)}`}
      />
      <div className="pointer-events-none relative aspect-square overflow-hidden rounded-lg bg-[#111]">
        {album.cover_url ? (
          <Image
            alt=""
            className="object-cover"
            fill
            sizes="(min-width: 768px) 180px, 45vw"
            src={album.cover_url}
          />
        ) : (
          <AlbumArtFallback />
        )}
        <span className="absolute bottom-2 left-2">
          <SourceBadge provider={album.provider} variant="overlay" />
        </span>
      </div>
      <div className="absolute right-2 top-2 z-10 rounded-full bg-black/60 backdrop-blur-md">
        <LibraryButton
          compact
          id={album.id}
          inLibrary={album.in_library}
          kind="album"
          title={album.title}
        />
      </div>
      <p className="pointer-events-none mt-2.5 truncate text-sm font-semibold text-zinc-100">
        {album.title}
      </p>
      <p className="pointer-events-none flex items-center gap-2 truncate text-sm text-zinc-500">
        <span className="truncate">{album.artist.name}</span>
        {album.format && (
          <FormatBadge
            className="shrink-0"
            label={album.format.label}
            tone={formatTone(album.format)}
          />
        )}
      </p>
    </div>
  );
}

function ServiceResults({
  service,
  query,
}: {
  service: StreamingServiceSearch;
  query: string;
}) {
  const playback = useStreamingPlayback();
  if (service.error) {
    return (
      <section className="space-y-2">
        <ServiceHeading service={service} />
        <p className="text-sm text-amber-200/90">{service.error}</p>
      </section>
    );
  }
  const results = service.results;
  if (
    !results ||
    (!results.albums.length &&
      !results.tracks.length &&
      !results.artists.length)
  ) {
    return (
      <section className="space-y-2">
        <ServiceHeading service={service} />
        <p className="text-sm text-zinc-500">
          No {service.name} matches for “{query}”.
        </p>
      </section>
    );
  }
  const tracks: StreamingTrack[] = results.tracks.slice(0, 6);
  return (
    <section className="space-y-5">
      <ServiceHeading service={service} />
      {results.artists.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {results.artists.slice(0, 6).map((artist) => (
            <Link
              className="inline-flex items-center gap-2 rounded-full border border-white/10 py-1 pl-1 pr-3 text-sm text-zinc-200 hover:border-white/25 hover:text-white"
              href={`/artist?id=${encodeURIComponent(artist.id)}`}
              key={artist.id}
            >
              <span className="relative flex h-7 w-7 items-center justify-center overflow-hidden rounded-full bg-white/8">
                {artist.picture_url ? (
                  <Image
                    alt=""
                    className="object-cover"
                    fill
                    sizes="28px"
                    src={artist.picture_url}
                  />
                ) : (
                  <UserRound className="h-3.5 w-3.5 text-zinc-500" />
                )}
              </span>
              {artist.name}
            </Link>
          ))}
        </div>
      )}
      {results.albums.length > 0 && (
        <div>
          <h3 className="mb-3 text-sm font-semibold text-zinc-300">Albums</h3>
          <div className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-3 md:grid-cols-4">
            {results.albums.slice(0, 8).map((album) => (
              <AlbumTile album={album} key={album.id} />
            ))}
          </div>
        </div>
      )}
      {tracks.length > 0 && (
        <div>
          <h3 className="mb-2 text-sm font-semibold text-zinc-300">Tracks</h3>
          <div className="space-y-0.5">
            {tracks.map((track, index) => (
              <StreamingTrackRow
                active={playback.currentSongId === track.id}
                index={index}
                key={track.id}
                onPlay={() => playback.playTracks(tracks, index)}
                playing={
                  playback.isPlaying && playback.currentSongId === track.id
                }
                showAlbum
                showFormat
                track={{ ...track, track_number: index + 1 }}
              />
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

function ServiceHeading({ service }: { service: StreamingServiceSearch }) {
  return (
    <h2 className="flex items-center gap-3 border-b border-white/8 pb-2 text-lg font-semibold text-white">
      From{" "}
      <SourceBadge
        className="text-[0.9375rem] text-white"
        provider={service.provider}
      />
    </h2>
  );
}

function LibraryItemRow({ item }: { item: StreamingLibraryItem }) {
  const playback = useStreamingPlayback();
  const href =
    item.kind === "album"
      ? `/album?id=${encodeURIComponent(item.id)}`
      : item.item && "album" in item.item && item.item.album
        ? `/album?id=${encodeURIComponent(item.item.album.id)}`
        : null;
  const track =
    item.kind === "track" && item.item && "track_number" in item.item
      ? item.item
      : null;
  return (
    <article className="group relative flex items-center gap-4 rounded-lg border-b border-white/[0.07] px-2 py-3 hover:bg-white/2.5">
      {track ? (
        <button
          aria-label={`Play ${item.title}`}
          className="absolute inset-0 rounded-[inherit]"
          onClick={() => playback.playTracks([track])}
          type="button"
        />
      ) : (
        href && (
          <Link
            aria-label={`View ${item.title}`}
            className="absolute inset-0 rounded-[inherit]"
            href={href}
          />
        )
      )}
      <div className="pointer-events-none relative h-14 w-14 shrink-0 overflow-hidden rounded-md bg-white/4">
        {item.cover_url ? (
          <Image
            alt=""
            className="object-cover"
            fill
            sizes="56px"
            src={item.cover_url}
          />
        ) : (
          <AlbumArtFallback />
        )}
      </div>
      <div className="pointer-events-none min-w-0 flex-1">
        <p className="truncate text-[0.9375rem] font-medium text-zinc-100">
          {item.title}
        </p>
        <p className="mt-0.5 flex items-center gap-2 truncate text-sm text-zinc-500">
          <span className="capitalize">{item.kind}</span>·
          <span className="truncate">{item.artist_name}</span>
        </p>
      </div>
      <SourceBadge provider={item.provider} />
      {item.format_label && (
        <FormatBadge
          className="hidden sm:inline-flex"
          label={item.format_label}
          tone={formatToneFromLabel(item.format_label)}
        />
      )}
    </article>
  );
}

/**
 * Streaming results for the search page: library-scoped streaming albums and
 * tracks, or every connected service's catalog.
 */
export default function StreamingSearchSections({
  query,
  scope,
}: {
  query: string;
  scope: SearchScope;
}) {
  const {
    connected,
    sandbox,
    isPending: providersPending,
  } = useStreamingProviders();
  const hasServices = connected.length > 0;
  const library = useQuery({
    queryKey: ["streaming", "library-search", query],
    queryFn: async () => {
      const [albums, tracks] = await Promise.all([
        getStreamingLibrary({ kind: "album", q: query, limit: 20 }),
        getStreamingLibrary({ kind: "track", q: query, limit: 20 }),
      ]);
      return [...albums.items, ...tracks.items];
    },
    enabled: Boolean(query) && scope === "library",
  });
  const catalog = useQuery({
    queryKey: ["streaming", "search", query],
    queryFn: () => searchStreaming(query, { limit: 12 }),
    enabled: Boolean(query) && scope === "everywhere" && hasServices,
    staleTime: 60_000,
  });

  if (!query) return null;

  if (scope === "library") {
    const items = library.data ?? [];
    if (!items.length) return null;
    return (
      <section className="space-y-2">
        <h2 className="text-lg font-semibold text-white">
          Streaming albums and tracks in your library
        </h2>
        {items.map((item) => (
          <LibraryItemRow item={item} key={`${item.kind}-${item.id}`} />
        ))}
      </section>
    );
  }

  if (!hasServices) {
    if (providersPending) return null;
    return (
      <p className="rounded-xl border border-white/8 px-4 py-3 text-sm text-zinc-400">
        Connect TIDAL or Qobuz in{" "}
        <Link
          className="text-white underline-offset-4 hover:underline"
          href="/settings?tab=services"
        >
          Settings, Services
        </Link>{" "}
        to search their catalogs here too.
      </p>
    );
  }

  return (
    <div className="space-y-10">
      {sandbox && (
        <p className="text-xs text-amber-200/80">
          Sandbox mode: service results are recorded responses.
        </p>
      )}
      {catalog.isFetching && !catalog.data && (
        <p className="flex items-center gap-2 text-sm text-zinc-500">
          <Loader2 className="h-4 w-4 animate-spin" /> Searching{" "}
          {connected.map((provider) => provider.name).join(" and ")}…
        </p>
      )}
      {catalog.isError && (
        <p className="text-sm text-zinc-400">
          Streaming services could not be searched.{" "}
          <button
            className="text-white underline-offset-4 hover:underline"
            onClick={() => void catalog.refetch()}
            type="button"
          >
            Try again
          </button>
        </p>
      )}
      {catalog.data?.services.map((service) => (
        <ServiceResults
          key={service.provider}
          query={query}
          service={service}
        />
      ))}
    </div>
  );
}
