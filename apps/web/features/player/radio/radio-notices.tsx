"use client";

import {
  radioComingUpIn,
  radioSectionStart,
  radioSkipReasons,
} from "@parson/music-sdk";
import { defaultCover } from "@/lib/images/default-cover";
import getBaseURL from "@/lib/api/server-url";
import { X } from "lucide-react";
import Image from "next/image";
import { useEffect, useState } from "react";
import { usePlayer, usePlayerClock } from "../player-api";
import { RadioThumbs } from "./radio-controls";

const coverUrl = (path?: string) =>
  path
    ? `${getBaseURL()}/media/images/${encodeURIComponent(path)}`
    : defaultCover;

const countdown = (seconds: number) =>
  `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;

const card =
  "pointer-events-auto w-full rounded-2xl border border-white/10 bg-[#161618]/95 p-4 text-white shadow-[0_24px_80px_rgba(0,0,0,0.7)] backdrop-blur-xl motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-bottom-2";

/** Stack for Radio notices, above the player bar. */
export function RadioNotices() {
  return (
    <div className="pointer-events-none fixed bottom-42 right-3 z-60 flex w-[min(380px,calc(100vw-24px))] flex-col gap-2 md:bottom-36 md:right-6">
      <RadioSkipPrompt />
      <RadioComingUp />
    </div>
  );
}

/** "Coming up in 0:15": the next Radio pick, with thumbs. */
export function RadioComingUp() {
  const { currentIndex, duration, looping, queue, radio } = usePlayer();
  const currentTime = usePlayerClock();
  const [dismissed, setDismissed] = useState("");
  const nextIndex = currentIndex + 1;
  const next = queue[nextIndex];
  const sectionStart = radioSectionStart(
    queue.map((item) => item.origin),
    currentIndex,
  );
  const remaining = radioComingUpIn({
    currentTime,
    duration,
    looping,
    nextIsRadioPick: sectionStart >= 0 && sectionStart <= nextIndex,
  });
  const key = next ? `${currentIndex}:${next.song.id}` : "";
  if (
    !next ||
    remaining === null ||
    !radio.enabled ||
    !radio.comingUpNotice ||
    dismissed === key
  ) {
    return null;
  }
  const title = next.song.name || "Untitled song";
  const artist = next.song.artist || next.artist.name || "Unknown artist";
  return (
    <section aria-label="Coming up on Radio" className={card} role="status">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm font-semibold">
          Coming up in{" "}
          <span className="tabular-nums">{countdown(remaining)}</span>
        </p>
        <button
          aria-label="Hide coming up"
          className="-mr-1 grid h-7 w-7 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
          onClick={() => setDismissed(key)}
          type="button"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <div className="mt-3 flex items-center gap-3">
        <div className="relative h-14 w-14 shrink-0 overflow-hidden rounded-md bg-zinc-900">
          <Image
            alt=""
            className="object-cover"
            fill
            sizes="56px"
            src={coverUrl(next.album.cover_url)}
          />
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[15px] font-semibold">{title}</p>
          <p className="truncate text-sm text-zinc-400">{artist}</p>
          {next.reason && (
            <p className="truncate text-xs text-zinc-500">
              Radio · {next.reason}
            </p>
          )}
        </div>
        <RadioThumbs
          index={nextIndex}
          label={title}
          liked={next.liked}
          size="lg"
          solid
        />
      </div>
    </section>
  );
}

/** Optional "Tell us why" after skipping a Radio pick early. */
export function RadioSkipPrompt() {
  const { radio } = usePlayer();
  const { answerSkip, dismissSkip, pendingSkip } = radio;
  useEffect(() => {
    if (!pendingSkip) return;
    const timeout = window.setTimeout(dismissSkip, 12_000);
    return () => window.clearTimeout(timeout);
  }, [dismissSkip, pendingSkip]);
  if (!pendingSkip) {
    return null;
  }
  return (
    <section aria-label="Why did you skip?" className={card}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <div className="relative h-10 w-10 shrink-0 overflow-hidden rounded-md bg-zinc-900">
            <Image
              alt=""
              className="object-cover"
              fill
              sizes="40px"
              src={pendingSkip.cover}
            />
          </div>
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold">
              Skipped {pendingSkip.song.name || "this track"}
            </p>
            <p className="text-xs text-zinc-500">
              Tell us why, and Radio will learn. Optional.
            </p>
          </div>
        </div>
        <button
          aria-label="No reason"
          className="-mr-1 grid h-7 w-7 shrink-0 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
          onClick={dismissSkip}
          type="button"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <div className="mt-3 flex flex-wrap gap-1.5">
        {radioSkipReasons.map((reason) => (
          <button
            className="rounded-full border border-white/12 px-3 py-1.5 text-xs font-medium text-zinc-200 transition-colors hover:border-white/30 hover:bg-white/8 hover:text-white"
            key={reason.id}
            onClick={() => answerSkip(reason.id)}
            type="button"
          >
            {reason.label}
          </button>
        ))}
      </div>
    </section>
  );
}
