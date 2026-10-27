"use client";

import {
  getStreamDelivery,
  parseStreamingId,
  playIdFromStreamUrl,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, AudioLines, CheckCircle2 } from "lucide-react";
import { useContext, useEffect } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { PlayerContext } from "@/features/player/player-api";
import { cn } from "@/lib/cn";
import { FormatBadge } from "./source-badge";
import { deliveryQueryKey, deliverySummary } from "./streaming-model";

const POLL_MS = 1500;
const MAX_POLLS = 20;

/**
 * The format a streaming track actually delivered. Parson reads it from the
 * stream's own bytes as they pass through the server, so the badge shows
 * what arrived, not just the quality that was asked for.
 *
 * With `play` (the id in the playing stream's URL) it asks about exactly
 * that stream: a new track, a replay or a quality change starts a new play
 * with no report until its own bytes arrive, so the badge never shows an
 * earlier stream's format. `frames` counts first-frame decodes of that
 * stream and re-reads the report each time.
 */
export function useStreamDelivery(
  songId: string | null | undefined,
  play?: string | null,
  frames = 0,
) {
  const parsed = parseStreamingId(songId);
  const query = useQuery({
    queryKey: deliveryQueryKey(songId, play),
    queryFn: () => getStreamDelivery(songId ?? "", play),
    enabled: Boolean(parsed),
    staleTime: 10_000,
    refetchInterval: (query) =>
      query.state.data || query.state.dataUpdateCount >= MAX_POLLS
        ? false
        : POLL_MS,
  });
  const { refetch } = query;
  useEffect(() => {
    if (frames > 0 && parsed) void refetch();
  }, [frames, parsed, refetch]);
  return query;
}

export function DeliveredFormat({
  songId,
  className,
  align = "start",
}: {
  songId: string;
  className?: string;
  align?: "start" | "center" | "end";
}) {
  const player = useContext(PlayerContext);
  // Only the track now playing has a stream to ask about.
  const playing = player?.song.id === songId ? player : null;
  const play = playing ? playIdFromStreamUrl(playing.stream.url) : null;
  const { data } = useStreamDelivery(songId, play, playing?.stream.frames ?? 0);
  if (!data) return null;
  const summary = deliverySummary(data, playing?.song.duration);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={`Signal path: delivered ${summary.badge}`}
        className={cn(
          "inline-flex shrink-0 items-center gap-1 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-white/40",
          className,
        )}
      >
        <FormatBadge label={summary.badge} tone={summary.tone} />
        {summary.warning && (
          <AlertTriangle
            aria-hidden="true"
            className="h-3.5 w-3.5 text-amber-300"
          />
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent align={align} className="w-72 p-3" side="top">
        <DropdownMenuLabel className="flex items-center gap-2 px-0 pt-0 text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">
          <AudioLines aria-hidden="true" className="h-3.5 w-3.5" />
          Signal path
        </DropdownMenuLabel>
        <ul className="mt-1 space-y-1.5 text-[13px] leading-5 text-zinc-200">
          {summary.lines.map((line) => (
            <li className="flex items-start gap-2" key={line}>
              <CheckCircle2
                aria-hidden="true"
                className="mt-0.5 h-3.5 w-3.5 shrink-0 text-zinc-500"
              />
              {line}
            </li>
          ))}
        </ul>
        {summary.warning && (
          <>
            <DropdownMenuSeparator className="my-2.5" />
            <p className="flex items-start gap-2 text-[13px] leading-5 text-amber-200">
              <AlertTriangle
                aria-hidden="true"
                className="mt-0.5 h-3.5 w-3.5 shrink-0"
              />
              {summary.warning}. Shown as it arrived.
            </p>
          </>
        )}
        <DropdownMenuSeparator className="my-2.5" />
        <p className="text-[11.5px] leading-4 text-zinc-500">
          Read from the stream itself as Mujin Music relays it, not from the
          service&apos;s catalog.
        </p>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
