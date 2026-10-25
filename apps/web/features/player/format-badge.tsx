"use client";

import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { useQuery } from "@tanstack/react-query";
import { ArrowDown, Check } from "lucide-react";
import {
  describeAudioFormat,
  getSongDelivery,
  type DeliveryPlan,
} from "@parson/music-sdk";
import type { AudioProperties } from "@parson/music-sdk/types";
import { AudioQualityBadge } from "@/features/library/audio-quality-badge";
import { cn } from "@/lib/cn";
import {
  PATH_QUALITY_DOT,
  deliveryPathRows,
  pathQuality,
  pathQualityLabel,
} from "./delivery-path";
import { browserPlaybackCapabilities } from "./playback-capabilities";

/** Fetches the server's delivery plan for the current track on this player. */
export function useSongDelivery(songId: string, bitrate: number) {
  return useQuery({
    queryKey: ["song-delivery", songId, bitrate],
    queryFn: () =>
      getSongDelivery(songId, browserPlaybackCapabilities(), { bitrate }),
    enabled: Boolean(songId),
    staleTime: 5 * 60_000,
    retry: 1,
  }).data?.plan;
}

/**
 * The now-playing format chip with a quality light; opens the signal path,
 * which shows the source, each server step and this device.
 */
export function FormatBadge({
  audio,
  bitrate,
  casting = false,
  className,
  presetLabel,
  songId,
}: {
  audio?: AudioProperties | null;
  bitrate: number;
  casting?: boolean;
  className?: string;
  presetLabel?: string | null;
  songId: string;
}) {
  const plan = useSongDelivery(casting ? "" : songId, bitrate);
  const summary =
    describeAudioFormat(audio) ??
    (plan ? describeAudioFormat(planSourceAudio(plan)) : null);
  if (!summary) return null;
  const rows = deliveryPathRows(plan, { presetLabel, casting });
  const quality = rows.length ? pathQuality(rows) : null;
  return (
    <DropdownMenuPrimitive.Root>
      <DropdownMenuPrimitive.Trigger
        aria-label={`Signal path: ${quality ? pathQualityLabel(quality) : "loading"}`}
        className={cn(
          "group inline-flex min-w-0 items-center gap-1.5 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-white/40",
          className,
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            "h-1.5 w-1.5 shrink-0 rounded-full transition-colors",
            quality ? PATH_QUALITY_DOT[quality] : "bg-zinc-600",
          )}
        />
        <AudioQualityBadge
          className="transition-colors group-hover:border-white/30"
          summary={summary}
        />
      </DropdownMenuPrimitive.Trigger>
      <DropdownMenuPrimitive.Portal>
        <DropdownMenuPrimitive.Content
          align="start"
          className="z-100 w-[min(360px,calc(100vw-32px))] rounded-2xl border border-white/10 bg-[#0d0d0f]/95 p-4 text-white shadow-[0_24px_80px_rgba(0,0,0,0.75)] backdrop-blur-xl"
          collisionPadding={16}
          side="top"
          sideOffset={12}
        >
          <SignalPathPanel plan={plan} rows={rows} />
        </DropdownMenuPrimitive.Content>
      </DropdownMenuPrimitive.Portal>
    </DropdownMenuPrimitive.Root>
  );
}

function planSourceAudio(plan: DeliveryPlan): AudioProperties {
  const { name: _name, ...source } = plan.source;
  return source;
}

function SignalPathPanel({
  plan,
  rows,
}: {
  plan: DeliveryPlan | undefined;
  rows: ReturnType<typeof deliveryPathRows>;
}) {
  if (!plan) {
    return <p className="text-sm text-zinc-400">Reading the signal path…</p>;
  }
  const quality = pathQuality(rows);
  return (
    <div className="space-y-4">
      <header className="flex items-center justify-between gap-3">
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">
            Signal path
          </p>
          <p className="mt-0.5 flex items-center gap-2 text-base font-semibold">
            <span
              aria-hidden="true"
              className={cn("h-2 w-2 rounded-full", PATH_QUALITY_DOT[quality])}
            />
            {pathQualityLabel(quality)}
          </p>
        </div>
        {plan.gapless ? (
          <span className="inline-flex items-center gap-1 rounded-full border border-white/10 px-2 py-0.5 text-[11px] text-zinc-300">
            <Check className="h-3 w-3" /> Gapless
          </span>
        ) : null}
      </header>
      <ol className="relative space-y-3">
        {rows.map((row, index) => (
          <li className="relative flex gap-3" key={row.key}>
            <div className="flex w-3 flex-col items-center pt-1.5">
              <span
                aria-hidden="true"
                className={cn(
                  "h-2 w-2 shrink-0 rounded-full",
                  PATH_QUALITY_DOT[row.quality],
                )}
              />
              {index < rows.length - 1 ? (
                <span
                  aria-hidden="true"
                  className="mt-1 w-px flex-1 bg-linear-to-b from-white/20 to-white/5"
                />
              ) : null}
            </div>
            <div className="min-w-0 pb-0.5">
              <p className="text-[11px] font-medium uppercase tracking-[0.1em] text-zinc-500">
                {row.title}
              </p>
              <p className="text-sm leading-5 text-zinc-100">{row.detail}</p>
            </div>
          </li>
        ))}
      </ol>
      <p className="flex items-start gap-2 rounded-xl bg-white/4 p-3 text-xs leading-5 text-zinc-400">
        <ArrowDown className="mt-0.5 h-3.5 w-3.5 shrink-0 text-zinc-500" />
        {plan.reason}
      </p>
    </div>
  );
}
