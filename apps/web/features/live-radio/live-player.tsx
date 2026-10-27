"use client";

import {
  liveStationIdFromSongId,
  liveTitleText,
  type LiveNowPlaying,
} from "@parson/music-sdk";
import { cn } from "@/lib/cn";
import { LiveBadge, QualityBadge, SaveStationButton } from "./station-ui";
import { useLiveStation } from "./use-live-radio";

function clock(seconds: number) {
  const total = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = String(total % 60).padStart(2, "0");
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
}

/**
 * Replaces the seek bar while a live station plays: there is nothing to
 * seek, so it shows that the stream is live and how long it has played.
 */
export function LiveTimeline({
  className,
  currentTime,
  isPlaying,
  playing,
}: {
  className?: string;
  currentTime: number;
  isPlaying: boolean;
  playing?: LiveNowPlaying | null;
}) {
  return (
    <div
      aria-label="Live stream"
      className={cn(
        "items-center gap-2 text-[0.6875rem] text-zinc-500",
        className,
      )}
      role="group"
    >
      <span className="w-10 text-right tabular-nums">{clock(currentTime)}</span>
      <div className="relative h-1 flex-1 overflow-hidden rounded-full bg-white/10">
        <div
          className={cn(
            "absolute inset-y-0 left-0 w-full rounded-full bg-gradient-to-r from-rose-500/70 via-rose-400 to-rose-300",
            isPlaying ? "opacity-100" : "opacity-40",
          )}
        />
        {isPlaying && (
          <div className="absolute inset-y-0 w-1/4 animate-[live-sweep_2.4s_linear_infinite] bg-gradient-to-r from-transparent via-white/60 to-transparent motion-reduce:hidden" />
        )}
      </div>
      <LiveBadge />
      {playing?.quality && (
        <QualityBadge
          className="hidden lg:inline-flex"
          quality={playing.quality}
        />
      )}
    </div>
  );
}

/** The heart that saves the playing station to My Live Radio. */
export function LiveStationSave({ songId }: { songId: string }) {
  const station = useLiveStation(liveStationIdFromSongId(songId)).data?.station;
  if (!station) return null;
  return <SaveStationButton station={station} />;
}

/** "Artist — Title" from the stream, or the station's place and genres. */
export function liveArtistLine(
  playing: LiveNowPlaying | null | undefined,
  fallback: string,
): string {
  return liveTitleText(playing?.title) || fallback;
}
