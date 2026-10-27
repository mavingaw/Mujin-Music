"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  findLyrics,
  lyricsEmptyMessage,
  lyricsFailureKind,
  parseLrc,
  type LyricsSourceKind,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Loader2, Play } from "lucide-react";
import Image from "next/image";
import { LyricsSourceChip } from "./lyrics-view";
import type { QueueInput } from "./player-model";

/** Dispatched by "View lyrics" in a track's menu; the player bar answers. */
export const VIEW_LYRICS_EVENT = "mujin:view-lyrics";

export type ViewLyricsRequest = {
  songId: string;
  title: string;
  artist: string;
  cover?: string;
  /** Lets the sheet play the track and follow along. */
  item?: QueueInput;
};

export function requestViewLyrics(request: ViewLyricsRequest) {
  window.dispatchEvent(
    new CustomEvent<ViewLyricsRequest>(VIEW_LYRICS_EVENT, { detail: request }),
  );
}

function stamp(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const rest = Math.floor(seconds % 60);
  return `${minutes}:${rest < 10 ? "0" : ""}${rest}`;
}

/**
 * Lyrics of a track that is not playing: the words with their time stamps,
 * where they came from, and "Play and follow along", which plays the track
 * and opens the full-screen lyrics.
 */
export function LyricsSheet({
  onFollowAlong,
  onOpenChange,
  request,
}: {
  onFollowAlong: (request: ViewLyricsRequest) => void;
  onOpenChange: (open: boolean) => void;
  request: ViewLyricsRequest;
}) {
  const lyrics = useQuery({
    queryKey: ["lyrics", request.songId],
    queryFn: () => findLyrics(request.songId),
    retry: false,
    staleTime: 30 * 60_000,
  });
  const timed = parseLrc(lyrics.data?.syncedLyrics).filter((line) =>
    line.text.trim(),
  );
  const plain = lyrics.data?.plainLyrics;
  const failure = lyrics.isError ? lyricsFailureKind(lyrics.error) : null;
  const empty = lyricsEmptyMessage(
    failure ?? "missing",
    Boolean(lyrics.data?.instrumental),
  );
  const hasWords = timed.length > 0 || Boolean(plain);
  return (
    <Dialog onOpenChange={onOpenChange} open>
      <DialogContent className="max-h-[85dvh] gap-0 overflow-hidden border-white/[0.08] bg-mujin-raised p-0 sm:max-w-xl">
        <div className="flex items-center gap-4 border-b border-white/[0.06] p-5 pr-14">
          {request.cover && (
            <span className="relative h-14 w-14 shrink-0 overflow-hidden rounded-[4px] bg-mujin-surface-1">
              <Image
                alt=""
                className="object-cover"
                fill
                sizes="56px"
                src={request.cover}
              />
            </span>
          )}
          <div className="min-w-0">
            <p className="mj-eyebrow text-[10px]!">Lyrics</p>
            <DialogTitle className="mj-title truncate text-[20px] text-mujin-text">
              {request.title}
            </DialogTitle>
            <DialogDescription className="truncate text-[13px] text-mujin-text-secondary">
              {request.artist}
            </DialogDescription>
          </div>
        </div>
        <div className="app-shell-scroll max-h-[55dvh] overflow-y-auto px-5 py-5">
          {lyrics.isPending ? (
            <p
              className="flex items-center gap-2 text-sm text-mujin-text-secondary"
              role="status"
            >
              <Loader2 className="h-4 w-4 motion-safe:animate-spin" />
              Finding lyrics in the file, beside it, then online…
            </p>
          ) : timed.length ? (
            <ol className="space-y-1.5">
              {timed.map((line, index) => (
                <li
                  className="grid grid-cols-[3rem_minmax(0,1fr)] items-baseline gap-3"
                  key={`${line.time}-${index}`}
                >
                  <span className="mj-tnum text-right font-mono text-[11px] text-mujin-text-tertiary">
                    {stamp(line.time)}
                  </span>
                  <span className="text-[15px] leading-6 text-mujin-text">
                    {line.text}
                  </span>
                </li>
              ))}
            </ol>
          ) : plain ? (
            <p className="whitespace-pre-line text-[15px] leading-7 text-mujin-text">
              {plain}
            </p>
          ) : (
            <div role="status">
              <p className="mj-title text-[20px] text-mujin-text">
                {empty.title}
              </p>
              <p className="mt-1.5 text-[13px] leading-5 text-mujin-text-secondary">
                {empty.detail}
              </p>
              {empty.retry && (
                <button
                  className="mt-4 rounded-full border border-white/15 px-4 py-2 text-[12.5px] font-semibold text-mujin-text hover:bg-white/[0.06]"
                  onClick={() => void lyrics.refetch()}
                  type="button"
                >
                  Try again
                </button>
              )}
            </div>
          )}
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-white/[0.06] px-5 py-4">
          {hasWords ? (
            <LyricsSourceChip
              source={(lyrics.data?.source ?? null) as LyricsSourceKind | null}
              timed={timed.length > 0}
            />
          ) : (
            <span />
          )}
          {request.item && (
            <button
              className="mj-primary inline-flex h-9 items-center gap-2 rounded-full px-4 text-[13px] font-semibold"
              onClick={() => onFollowAlong(request)}
              type="button"
            >
              <Play className="h-3.5 w-3.5 fill-current" />
              {timed.length ? "Play and follow along" : "Play"}
            </button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
