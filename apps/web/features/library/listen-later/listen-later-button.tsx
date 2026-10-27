"use client";

import { cn } from "@/lib/cn";
import type { ListenLaterEntity } from "@parson/music-sdk";
import { ClockCheck, ClockPlus } from "lucide-react";
import { useChangeListenLater, useListenLaterSaved } from "./use-listen-later";

/** Round "Listen later" toggle for album and artist headers. */
export default function ListenLaterButton({
  className,
  entityId,
  entityType,
  name,
}: {
  className?: string;
  entityId: string;
  entityType: ListenLaterEntity;
  name?: string;
}) {
  const saved = useListenLaterSaved(entityType, entityId);
  const change = useChangeListenLater();
  const isSaved = Boolean(saved.data);
  const label = isSaved ? "Remove from Listen Later" : "Listen later";
  return (
    <button
      aria-label={label}
      aria-pressed={isSaved}
      className={cn(
        "grid h-10 w-10 place-items-center rounded-full border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/40 disabled:opacity-50",
        isSaved
          ? "border-mujin-copper/45 bg-mujin-copper/15 text-mujin-copper-hover hover:bg-mujin-copper/25"
          : "border-white/15 text-zinc-300 hover:border-white/35 hover:text-white",
        className,
      )}
      disabled={!entityId || saved.isPending || change.isPending}
      onClick={() =>
        change.mutate({
          entityType,
          entityIds: [entityId],
          save: !isSaved,
          name,
        })
      }
      title={label}
      type="button"
    >
      {isSaved ? (
        <ClockCheck className="h-[18px] w-[18px]" />
      ) : (
        <ClockPlus className="h-[18px] w-[18px]" />
      )}
    </button>
  );
}
