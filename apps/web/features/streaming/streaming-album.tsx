"use client";

import {
  STREAMING_PROVIDER_NAMES,
  getStreamingAlbum,
  parseStreamingId,
  type StreamingAlbum,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { ExternalLink, ListEnd, Pause, Play } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { usePageTitle } from "@/components/app/title-metadata";
import EntityPageState from "@/features/library/entity-page-state";
import { AlbumArtFallback } from "@/lib/images/image-url";
import {
  AlbumVersions,
  BetterVersionNotice,
  streamingAlbumSubject,
} from "./album-versions";
import { PreferredPlayCaption, usePreferredAlbumPlay } from "./preferred-play";
import { FormatBadge, SourceBadge } from "./source-badge";
import { LibraryButton } from "./library-button";
import { StartRadioButton } from "./radio-button";
import { AlbumCredits, AwardChips, EditorialBlock } from "./streaming-notes";
import { formatTone, totalDurationLabel } from "./streaming-model";
import { StreamingTrackRow } from "./streaming-track-row";
import { SandboxNotice } from "./services-settings";
import { useStreamingPlayback, useStreamingProviders } from "./use-streaming";

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
  const subject = data ? streamingAlbumSubject(data) : null;
  const playThisCopy = () => {
    if (data) playback.playAlbum(data);
  };
  const preferred = usePreferredAlbumPlay(subject, playThisCopy);

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

  const isAlbumPlaying = preferred.playing;
  const albumSubject = streamingAlbumSubject(data);
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
            <h1 className="mb-4 text-balance text-4xl mj-display leading-[0.98] text-mujin-text sm:text-5xl">
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
            <AwardChips awards={data.awards} />
          </div>
        </div>

        <div className="mb-7 flex flex-wrap items-center gap-4">
          <button
            aria-label={`${isAlbumPlaying ? "Pause" : "Play"} ${data.title}`}
            className="flex h-12 w-12 items-center justify-center rounded-full mj-primary motion-safe:hover:scale-105 hover:bg-zinc-100 disabled:opacity-70"
            disabled={preferred.starting}
            onClick={() => void preferred.play()}
            type="button"
          >
            {isAlbumPlaying ? (
              <Pause className="h-6 w-6 fill-current" />
            ) : (
              <Play className="ml-1 h-6 w-6 fill-current" />
            )}
          </button>
          <PreferredPlayCaption
            onPlayThisCopy={playThisCopy}
            play={preferred}
          />
          <LibraryButton
            id={data.id}
            inLibrary={data.in_library}
            kind="album"
            title={data.title}
          />
          <StartRadioButton id={data.id} label={data.title} seed="album" />
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

        <BetterVersionNotice subject={albumSubject} />

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

        {data.review && (
          <EditorialBlock text={data.review} title="About this album" />
        )}

        <AlbumCredits album={data} />

        <AlbumVersions subject={albumSubject} />

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
