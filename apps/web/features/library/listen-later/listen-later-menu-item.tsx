"use client";

import { ContextMenuItem } from "@/components/ui/context-menu";
import type { ListenLaterEntity } from "@parson/music-sdk";
import { ClockCheck, ClockPlus, Loader2 } from "lucide-react";
import { useChangeListenLater, useListenLaterSaved } from "./use-listen-later";

/**
 * "Listen later" / "Remove from Listen Later" for a context menu. Menu
 * content mounts only while open, so the lookup runs when it is needed.
 */
export function ListenLaterMenuItem({
  entityId,
  entityType,
  name,
}: {
  entityId: string;
  entityType: ListenLaterEntity;
  name?: string;
}) {
  const saved = useListenLaterSaved(entityType, entityId);
  const change = useChangeListenLater();
  const isSaved = Boolean(saved.data);
  return (
    <ContextMenuItem
      disabled={saved.isPending || change.isPending}
      onSelect={() =>
        change.mutate({
          entityType,
          entityIds: [entityId],
          save: !isSaved,
          name,
        })
      }
    >
      {saved.isPending ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : isSaved ? (
        <ClockCheck className="h-4 w-4" />
      ) : (
        <ClockPlus className="h-4 w-4" />
      )}
      {isSaved ? "Remove from Listen Later" : "Listen later"}
    </ContextMenuItem>
  );
}
