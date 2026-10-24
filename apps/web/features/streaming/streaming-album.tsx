"use client";

import {
  STREAMING_PROVIDER_NAMES,
  getStreamingAlbum,
  parseStreamingId,
  searchLibrary,
  searchStreaming,
  type StreamingAlbum,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { ExternalLink, ListEnd, Pause, Play } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { usePageTitle } from "@/components/app/title-metadata";
import EntityPageState from "@/features/library/entity-page-state";
import { AlbumArtFallback } from "@/lib/images/image-url";
import { FormatBadge, LocalSourceBadge, SourceBadge } from "./source-badge";
import { LibraryButton } from "./library-button";
import { formatTone, totalDurationLabel } from "./streaming-model";
import { StreamingTrackRow } from "./streaming-track-row";
import { SandboxNotice } from "./services-settings";
import { useStreamingPlayback, useStreamingProviders } from "./use-streaming";

const comparable = (value: string) =>
  value
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/\(.*?\)|\[.*?\]/g, "")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

/** Other places the same album lives: the other service and local files. */
function AlbumVersions({ album }: { album: StreamingAlbum }) {
  const { connected } = useStreamingProviders();
  const others = connected
    .map((provider) => provider.id)
    .filter((provider) => provider !== album.provider);
  const query = `${album.artist.name} ${album.title}`;
  const versions = useQuery({
    queryKey: ["streaming", "versions", album.id, others.join(",")],
    queryFn: async () => {
      const [remote, local] = await Promise.all([
        others.length
          ? searchStreaming(query, { providers: others, limit: 10 })
          : Promise.resolve({ query, services: [] }),
        searchLibrary(album.title).catch(() => []),
      ]);
      const title = comparable(album.title);
      const artist = comparable(album.artist.name);
      return {
        streaming: remote.services.flatMap((service) =>
          (service.results?.albums ?? []).filter(
            (candidate) =>
              comparable(candidate.title) === title &&
              comparable(candidate.artist.name) === artist,
          ),
        ),
        local: local.filter(
          (item) =>
            item.item_type === "album" &&
            comparable(item.name) === title &&
            comparable(item.artist_object?.name ?? "") === artist,
        ),
      };
    },
    staleTime: 5 * 60_000,
  });
  const count =
    (versions.data?.streaming.length ?? 0) + (versions.data?.local.length ?? 0);
  if (!count) return null;
  return (
    <section className="mt-10">
      <h2 className="mb-3 text-lg font-semibold text-white">Versions</h2>
      <div className="divide-y divide-white/6 rounded-xl border border-white/8">
        <VersionRow
          current
          formatLabel={album.format?.label ?? null}
          href={`/album?id=${encodeURIComponent(album.id)}`}
          source={<SourceBadge provider={album.provider} />}
          title={album.title}
          tone={formatTone(album.format)}
        />
        {versions.data?.streaming.map((version) => (
          <VersionRow
            formatLabel={version.format?.label ?? null}
            href={`/album?id=${encodeURIComponent(version.id)}`}
            key={version.id}
            source={<SourceBadge provider={version.provider} />}
            title={version.title}
            tone={formatTone(version.format)}
          />
        ))}
        {versions.data?.local.map((version) => (
          <VersionRow
            formatLabel={null}
            href={`/album?id=${encodeURIComponent(version.id)}`}
            key={version.id}
            source={<LocalSourceBadge />}
            title={version.name}
            tone={null}
          />
        ))}
      </div>
    </section>
  );
}

function VersionRow({
  title,
  source,
  formatLabel,
  tone,
  href,
  current = false,
}: {
  title: string;
  source: React.ReactNode;
  formatLabel: string | null;
  tone: ReturnType<typeof formatTone>;
  href: string;
  current?: boolean;
}) {
  return (
    <Link
      aria-current={current ? "page" : undefined}
      className="flex items-center gap-4 px-4 py-3 text-sm transition-colors hover:bg-white/3"
      href={href}
    >
      <span className="w-28 shrink-0">{source}</span>
      <span className="min-w-0 flex-1 truncate text-zinc-200">{title}</span>
      {formatLabel && <FormatBadge label={formatLabel} tone={tone} />}
      {current && <span className="text-xs text-zinc-500">Viewing</span>}
    </Link>
  );
}

export default function StreamingAlbumDetails({ id }: { id: string }) {
  const parsed = parseStreamingId(id);
  const album = useQuery({
    queryKey: ["streaming", "album", id],
    queryFn: () => getStreamingAlbum(id),
    enabled: Boolean(parsed),
    retry: 1,
  });
  const { sandbox } = useStreamingProviders();
  const playback = useStreamingPlayback();
  const data = album.data;
  usePageTitle(data ? `${data.title} - ${data.artist.name}` : null);

  if (!parsed) return <EntityPageState kind="album" />;
  if (album.isPending) return <EntityPageState kind="album" loading />;
  if (album.isError || !data) {
    const message = (
      album.error as { response?: { data?: { message?: string } } } | null
    )?.response?.data?.message;
    return (
      <div className="mx-auto max-w-xl px-6 py-20 text-center">
        <p className="text-lg font-semibold text-white">
          This {STREAMING_PROVIDER_NAMES[parsed.provider]} album could not be
          loaded
        </p>
        <p className="mt-2 text-sm text-zinc-400">
          {message ?? "Check your connection and try again."}
        </p>
        <div className="mt-6 flex justify-center gap-3">
          <button
            className="rounded-full bg-white px-5 py-2 text-sm font-semibold text-black"
            onClick={() => void album.refetch()}
            type="button"
          >
            Try again
          </button>
          <Link
            className="rounded-full border border-white/15 px-5 py-2 text-sm font-semibold text-white"
            href="/settings?tab=services"
          >
            Open Services
          </Link>
        </div>
      </div>
    );
  }

  const isAlbumPlaying =
    playback.isPlaying && playback.currentAlbumId === data.id;
  const year = data.release_date?.slice(0, 4);
  const tone = formatTone(data.format);
  const discs = new Set(data.tracks.map((track) => track.disc_number)).size;

  return (
    <div className="relative text-zinc-50">
      <div className="relative mx-auto max-w-225 px-5 pb-24 pt-8 sm:px-7">
        {sandbox && (
          <div className="mb-6">
            <SandboxNotice compact />
          </div>
        )}
        <div className="mb-8 flex w-full flex-col items-start gap-6 sm:flex-row sm:items-end">
          <div className="relative h-44 w-44 shrink-0 overflow-hidden rounded-md border border-white/5 sm:h-52 sm:w-52">
            {data.cover_url ? (
              <Image
                alt={data.title}
                className="object-cover"
                fill
                priority
                sizes="208px"
                src={data.cover_url}
              />
            ) : (
              <AlbumArtFallback />
            )}
          </div>
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <SourceBadge provider={data.provider} />
              {data.format && (
                <FormatBadge label={data.format.label} tone={tone} />
              )}
              {data.explicit && (
                <span className="rounded-[5px] bg-white/10 px-1.5 py-0.5 text-[10.5px] font-semibold text-zinc-300">
                  E
                </span>
              )}
            </div>
            <h1 className="mb-4 text-balance text-4xl font-black leading-[0.98] text-white sm:text-5xl">
              {data.title}
              {data.version && (
                <span className="block text-2xl font-bold text-zinc-400">
                  {data.version}
                </span>
              )}
            </h1>
            <div className="flex flex-wrap items-center gap-2 text-sm font-medium text-zinc-300">
              <Link
                className="font-bold text-white hover:underline"
                href={`/artist?id=${encodeURIComponent(data.artist.id)}`}
              >
                {data.artist.name}
              </Link>
              {year && (
                <>
                  <span className="text-zinc-500">•</span>
                  <span>{year}</span>
                </>
              )}
              <span className="text-zinc-500">•</span>
              <span>
                {data.tracks.length || data.track_count} tracks,{" "}
                {totalDurationLabel(data.duration)}
              </span>
              {data.genre && (
                <>
                  <span className="text-zinc-500">•</span>
                  <span>{data.genre}</span>
                </>
              )}
            </div>
          </div>
        </div>

        <div className="mb-7 flex flex-wrap items-center gap-4">
          <button
            aria-label={`${isAlbumPlaying ? "Pause" : "Play"} ${data.title}`}
            className="flex h-12 w-12 items-center justify-center rounded-full bg-white text-black transition-all hover:scale-105 hover:bg-zinc-100"
            onClick={() =>
              isAlbumPlaying
                ? playback.playAlbum(data, playback.currentSongId)
                : playback.playAlbum(data)
            }
            type="button"
          >
            {isAlbumPlaying ? (
              <Pause className="h-6 w-6 fill-black text-black" />
            ) : (
              <Play className="ml-1 h-6 w-6 fill-black text-black" />
            )}
          </button>
          <LibraryButton
            id={data.id}
            inLibrary={data.in_library}
            kind="album"
            title={data.title}
          />
          <button
            aria-label="Add to queue"
            className="flex h-10 w-10 items-center justify-center rounded-full text-zinc-400 transition-colors hover:bg-white/8 hover:text-white"
            onClick={() => playback.queueAlbum(data)}
            title="Add to queue"
            type="button"
          >
            <ListEnd className="h-5 w-5" />
          </button>
          {data.web_url && (
            <a
              className="ml-auto inline-flex items-center gap-1.5 text-sm text-zinc-500 hover:text-zinc-200"
              href={data.web_url}
              rel="noreferrer"
              target="_blank"
            >
              Open in {STREAMING_PROVIDER_NAMES[data.provider]}
              <ExternalLink className="h-3.5 w-3.5" />
            </a>
          )}
        </div>

        <div className="mb-2 grid select-none grid-cols-[2rem_minmax(0,1fr)_auto_3rem] gap-3 border-b border-white/10 px-3 py-2 text-sm font-medium text-zinc-500 sm:gap-4 sm:px-4">
          <div className="text-center">#</div>
          <div>Track</div>
          <div />
          <div className="text-right">Time</div>
        </div>
        <div className="flex flex-col space-y-1">
          {data.tracks.map((track, index) => {
            const firstOfDisc =
              discs > 1 &&
              (index === 0 ||
                data.tracks[index - 1]?.disc_number !== track.disc_number);
            return (
              <div key={track.id}>
                {firstOfDisc && (
                  <p className="px-4 pb-1 pt-4 text-xs font-semibold uppercase tracking-wider text-zinc-500">
                    Disc {track.disc_number}
                  </p>
                )}
                <StreamingTrackRow
                  active={playback.currentSongId === track.id}
                  index={index}
                  onPlay={() => playback.playAlbum(data, track.id)}
                  playing={
                    playback.isPlaying && playback.currentSongId === track.id
                  }
                  showFormat={
                    Boolean(track.format) &&
                    track.format?.label !== data.format?.label
                  }
                  track={track}
                />
              </div>
            );
          })}
        </div>

        <AlbumVersions album={data} />

        <footer className="mt-10 space-y-1 text-xs text-zinc-500">
          {data.release_date && <p>Released {data.release_date}</p>}
          {data.label && <p>{data.label}</p>}
          {data.copyright && <p>{data.copyright}</p>}
          <p>
            Streams from {STREAMING_PROVIDER_NAMES[data.provider]} at the
            quality chosen in Settings, Services.
          </p>
        </footer>
      </div>
    </div>
  );
}
