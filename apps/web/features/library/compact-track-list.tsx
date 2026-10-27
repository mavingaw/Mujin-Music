"use client";

import { cn } from "@/lib/cn";
import getBaseURL from "@/lib/api/server-url";
import { defaultCover } from "@/lib/images/default-cover";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import SongMenu from "@/features/library/song-menu";
import { formatDuration } from "@/features/library/library-view-primitives";
import { catalogSongsToQueue } from "@/features/library/focus/focus-state";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { usePlayer } from "@/features/player/player-context";
import type { FocusSong } from "@parson/music-sdk";
import { AudioLines, Play } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useMemo, type ReactNode } from "react";
import { PlayNowSplit } from "@/features/player/play-now-split";
import {
  TimedLyricsMark,
  useLyricsAvailability,
} from "@/features/library/timed-lyrics-mark";

const QUALITY_LABEL: Record<string, string> = {
  hires: "Hi-Res",
  cd: "Lossless",
};

/** Hi-Res / Lossless badge, as on album pages. */
export function QualityBadge({
  quality,
  className,
}: {
  quality?: string | null;
  className?: string;
}) {
  const label = quality ? QUALITY_LABEL[quality] : undefined;
  if (!label) return null;
  return (
    <span
      className={cn(
        "rounded-[4px] border px-1 py-px text-[0.625rem] font-bold uppercase leading-3 tracking-wide",
        quality === "hires"
          ? "border-mujin-copper/45 text-mujin-copper"
          : "border-white/20 text-zinc-300",
        className,
      )}
      title={quality === "hires" ? "High resolution lossless" : "Lossless"}
    >
      {label}
    </span>
  );
}

/**
 * A compact list of library tracks with artwork, that plays the list from
 * the clicked row on the selected zone.
 */
export default function CompactTrackList({
  songs,
  trailing,
}: {
  songs: FocusSong[];
  /** Extra controls at the end of a row (remove buttons, dates). */
  trailing?: (song: FocusSong, index: number) => ReactNode;
}) {
  const playback = useZonePlayback();
  const player = usePlayer();
  const queue = catalogSongsToQueue(songs);
  const lyrics = useLyricsAvailability(
    useMemo(() => songs.map((song) => song.id), [songs]),
  );
  return (
    <ol className="divide-y divide-white/[0.04] overflow-hidden rounded-xl border border-white/8 bg-white/[0.015]">
      {songs.map((song, index) => {
        const current = player.song.id === song.id;
        const cover =
          getLibraryImageUrl(song.coverPath, getBaseURL) ?? defaultCover;
        return (
          <SongMenu
            album_cover={song.coverPath}
            album_id={song.albumId}
            album_name={song.albumName}
            artist_id={song.artistId}
            artist_name={song.artistName}
            key={`${song.id}-${index}`}
            song_id={song.id}
            song_name={song.name}
          >
            <li className="group flex items-center gap-3 px-3 py-2 transition-colors hover:bg-white/[0.04]">
              <button
                aria-label={`Play ${song.name}`}
                className="relative h-10 w-10 shrink-0 overflow-hidden rounded-md bg-zinc-900"
                onClick={() => void playback.playQueue(queue, index)}
                type="button"
              >
                <Image
                  alt=""
                  className="h-full w-full object-cover"
                  height={80}
                  src={cover}
                  width={80}
                />
                <span className="absolute inset-0 grid place-items-center bg-black/55 opacity-0 transition-opacity group-hover:opacity-100">
                  {current && player.isPlaying ? (
                    <AudioLines className="h-4 w-4 text-white" />
                  ) : (
                    <Play className="h-4 w-4 fill-white text-white" />
                  )}
                </span>
              </button>
              <div className="min-w-0 flex-1">
                <p
                  className={cn(
                    "flex items-center gap-2 truncate text-sm font-medium",
                    current ? "text-mujin-copper" : "text-zinc-100",
                  )}
                >
                  <span className="truncate">{song.name}</span>
                  <QualityBadge quality={song.quality} />
                  <TimedLyricsMark availability={lyrics[song.id]} />
                </p>
                <p className="truncate text-xs text-zinc-500">
                  <Link
                    className="hover:text-white hover:underline"
                    href={`/artist?id=${encodeURIComponent(song.artistId)}`}
                  >
                    {song.artistName}
                  </Link>
                  {song.albumName && (
                    <>
                      {" · "}
                      <Link
                        className="hover:text-white hover:underline"
                        href={`/album?id=${encodeURIComponent(song.albumId)}`}
                      >
                        {song.albumName}
                      </Link>
                    </>
                  )}
                </p>
              </div>
              {queue[index] && (
                <PlayNowSplit
                  onPlayNow={() => void playback.playQueue(queue, index)}
                  target={{
                    kind: "track",
                    id: song.id,
                    name: song.name,
                    item: queue[index]!,
                  }}
                  variant="row"
                />
              )}
              {trailing?.(song, index)}
              <span className="w-12 shrink-0 text-right text-xs tabular-nums text-zinc-500">
                {formatDuration(song.durationSeconds)}
              </span>
            </li>
          </SongMenu>
        );
      })}
    </ol>
  );
}
