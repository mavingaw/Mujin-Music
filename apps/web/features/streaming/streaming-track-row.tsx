"use client";

import type { StreamingTrack } from "@parson/music-sdk";
import { Pause, Play } from "lucide-react";
import Link from "next/link";
import { cn } from "@/lib/cn";
import { ExplicitBadge } from "@/features/library/song-title";
import { FormatBadge } from "./source-badge";
import { LibraryButton } from "./library-button";
import { StartRadioButton } from "./radio-button";
import { formatTone } from "./streaming-model";

export function formatTrackDuration(duration: number) {
  const minutes = Math.floor(duration / 60);
  const seconds = Math.round(duration % 60);
  return `${minutes}:${seconds < 10 ? "0" : ""}${seconds}`;
}

/** A track row for streaming albums, artists and search results. */
export function StreamingTrackRow({
  track,
  index,
  active,
  playing,
  onPlay,
  showAlbum = false,
  showFormat = false,
}: {
  track: StreamingTrack;
  index: number;
  active: boolean;
  playing: boolean;
  onPlay: () => void;
  showAlbum?: boolean;
  showFormat?: boolean;
}) {
  const title = track.version
    ? `${track.title} (${track.version})`
    : track.title;
  const unavailable = !track.streamable;
  return (
    <div
      className={cn(
        "group relative grid min-h-14 grid-cols-[2rem_minmax(0,1fr)_auto_3rem] items-center gap-3 rounded-md px-3 py-2.5 text-sm transition-colors hover:bg-zinc-800/50 sm:gap-4 sm:px-4",
        active && "bg-white/2",
        unavailable && "opacity-50",
      )}
    >
      <button
        aria-label={`Play ${title}`}
        className="absolute inset-0 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-white/30 disabled:cursor-not-allowed"
        disabled={unavailable}
        onClick={onPlay}
        type="button"
      />
      <div className="pointer-events-none relative z-10 text-center text-zinc-400">
        <span className={active ? "block" : "group-hover:hidden"}>
          {active ? (
            playing ? (
              <Pause className="mx-auto h-4 w-4 fill-white text-white" />
            ) : (
              <Play className="mx-auto h-4 w-4 fill-white text-white" />
            )
          ) : (
            track.track_number || index + 1
          )}
        </span>
        {!active && (
          <Play className="absolute left-1/2 top-1/2 hidden h-4 w-4 -translate-x-1/2 -translate-y-1/2 fill-white text-white group-hover:block" />
        )}
      </div>
      <div className="pointer-events-none relative z-10 min-w-0">
        <p className="truncate font-medium text-white">{title}</p>
        <p className="mt-0.5 truncate text-zinc-400">
          {track.explicit && <ExplicitBadge />}
          <Link
            className="pointer-events-auto relative z-20 hover:text-white hover:underline"
            href={`/artist?id=${encodeURIComponent(track.artist.id)}`}
          >
            {track.artist.name}
          </Link>
          {showAlbum && track.album && (
            <>
              {" · "}
              <Link
                className="pointer-events-auto relative z-20 hover:text-white hover:underline"
                href={`/album?id=${encodeURIComponent(track.album.id)}`}
              >
                {track.album.title}
              </Link>
            </>
          )}
          {unavailable && " · Not available to stream"}
        </p>
      </div>
      <div className="relative z-20 flex items-center gap-2">
        {showFormat && track.format && (
          <FormatBadge
            className="hidden sm:inline-flex"
            label={track.format.label}
            tone={formatTone(track.format)}
          />
        )}
        {!unavailable && (
          <StartRadioButton
            className="hidden opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100 sm:inline-flex"
            compact
            id={track.id}
            label={track.title}
            seed="track"
          />
        )}
        <LibraryButton
          className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100 aria-pressed:opacity-100"
          compact
          id={track.id}
          inLibrary={track.in_library}
          kind="track"
          title={track.title}
        />
      </div>
      <div className="pointer-events-none relative z-10 text-right tabular-nums text-zinc-400">
        {formatTrackDuration(track.duration)}
      </div>
    </div>
  );
}
