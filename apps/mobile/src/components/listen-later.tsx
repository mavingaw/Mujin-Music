import {
  addToListenLater,
  isInListenLater,
  removeFromListenLater,
  type ListenLaterEntity,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ClockCheck, ClockPlus } from "lucide-react-native";

import { DrawerAction } from "@/components/action-drawer";
import { useSession } from "@/providers/session-provider";

export const listenLaterKey = ["listen-later"] as const;

/** Whether an album, track or artist is saved for later, and a toggle. */
export function useListenLater(
  entityType: ListenLaterEntity,
  entityId: string,
  enabled = true,
) {
  const session = useSession();
  const queryClient = useQueryClient();
  const online = session.phase === "ready";
  const saved = useQuery({
    queryKey: [...listenLaterKey, "saved", entityType, entityId],
    queryFn: () => isInListenLater(entityType, entityId),
    enabled: enabled && online && Boolean(entityId),
    staleTime: 30_000,
    retry: false,
  });
  const toggle = useMutation({
    mutationFn: async (save: boolean) => {
      if (save) await addToListenLater(entityType, [entityId]);
      else await removeFromListenLater(entityType, [entityId]);
      return save;
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: listenLaterKey }),
  });
  return {
    available: online,
    saved: Boolean(saved.data),
    pending: saved.isPending || toggle.isPending,
    toggle: () => toggle.mutate(!saved.data),
  };
}

/** "Listen later" / "Remove from Listen Later" in an action drawer. */
export function ListenLaterAction({
  entityId,
  entityType,
  onDone,
}: {
  entityId: string;
  entityType: ListenLaterEntity;
  onDone?: () => void;
}) {
  const listenLater = useListenLater(entityType, entityId);
  if (!listenLater.available) return null;
  return (
    <DrawerAction
      icon={listenLater.saved ? ClockCheck : ClockPlus}
      label={listenLater.saved ? "Remove from Listen Later" : "Listen later"}
      onPress={() => {
        listenLater.toggle();
        onDone?.();
      }}
    />
  );
}
