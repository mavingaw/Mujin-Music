"use client";

import {
  addToListenLater,
  isInListenLater,
  removeFromListenLater,
  type ListenLaterEntity,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { toast } from "sonner";

export const LISTEN_LATER_KEY = ["library", "listen-later"] as const;

const NOUNS: Record<ListenLaterEntity, string> = {
  album: "Album",
  track: "Track",
  artist: "Artist",
};

/** Whether one album, track or artist is saved for later. */
export function useListenLaterSaved(
  entityType: ListenLaterEntity,
  entityId: string,
  enabled = true,
) {
  return useQuery({
    queryKey: [...LISTEN_LATER_KEY, "saved", entityType, entityId],
    queryFn: () => isInListenLater(entityType, entityId),
    enabled: enabled && Boolean(entityId),
    staleTime: 30_000,
  });
}

export interface ListenLaterChange {
  entityType: ListenLaterEntity;
  entityIds: string[];
  save: boolean;
  /** Shown in the toast, e.g. the album title. */
  name?: string;
}

/** Saves or removes items, with an Undo and a shortcut to the list. */
export function useChangeListenLater() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const mutation = useMutation({
    mutationFn: async (change: ListenLaterChange) => {
      if (change.save) {
        await addToListenLater(change.entityType, change.entityIds);
      } else {
        await removeFromListenLater(change.entityType, change.entityIds);
      }
      return change;
    },
    onSuccess: async (change) => {
      await queryClient.invalidateQueries({ queryKey: LISTEN_LATER_KEY });
      const subject =
        change.entityIds.length === 1
          ? change.name
            ? `“${change.name}”`
            : NOUNS[change.entityType]
          : `${change.entityIds.length} items`;
      const undo = {
        label: "Undo",
        onClick: () => mutation.mutate({ ...change, save: !change.save }),
      };
      if (change.save) {
        toast.success(`${subject} saved to Listen Later`, {
          action: {
            label: "View",
            onClick: () => router.push("/listen-later"),
          },
          cancel: undo,
        });
      } else {
        toast.success(`${subject} removed from Listen Later`, {
          action: undo,
        });
      }
    },
    onError: () => toast("Could not update Listen Later."),
  });
  return mutation;
}
