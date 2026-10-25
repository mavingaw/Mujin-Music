"use client";

import { getStreamDelivery, parseStreamingId } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, AudioLines, CheckCircle2 } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/cn";
import { FormatBadge } from "./source-badge";
import { deliverySummary } from "./streaming-model";

const POLL_MS = 1500;
const MAX_POLLS = 20;

/**
 * The format a streaming track actually delivered. Parson reads it from the
 * stream's own bytes as they pass through the server, so the badge shows
 * what arrived, not just the quality that was asked for.
 */
export function useStreamDelivery(songId: string | null | undefined) {
  const parsed = parseStreamingId(songId);
  return useQuery({
    queryKey: ["streaming", "delivery", songId],
    queryFn: () => getStreamDelivery(songId ?? ""),
    enabled: Boolean(parsed),
    staleTime: 10_000,
    refetchInterval: (query) =>
      query.state.data || query.state.dataUpdateCount >= MAX_POLLS
        ? false
        : POLL_MS,
  });
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
  const { data } = useStreamDelivery(songId);
  if (!data) return null;
  const summary = deliverySummary(data);
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
              {summary.warning}, but this is what arrived.
            </p>
          </>
        )}
        <DropdownMenuSeparator className="my-2.5" />
        <p className="text-[11.5px] leading-4 text-zinc-500">
          Read from the stream itself as Parson relays it, not from the
          service&apos;s catalog.
        </p>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
