import {
  applyRadioBanChange,
  banFromRadio,
  emptyRadioBanIndex,
  getRadioBans,
  isRadioBanned,
  isSongExcludedByBans,
  radioBanActionLabel,
  radioBanIndex,
  radioBanKindLabel,
  unbanFromRadio,
  type BannableSong,
  type RadioBan,
  type RadioBanTarget,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban } from "lucide-react-native";
import { useCallback, useMemo, useState } from "react";

import { DrawerAction } from "@/components/action-drawer";
import { palette } from "@/constants/colors";
import { useSession } from "@/providers/session-provider";

export const radioBansKey = ["radio-bans"] as const;

export type BanTarget = RadioBanTarget & { name?: string | null };

/**
 * The listener's Radio bans. Bans live on the server, so Radio, Shuffle and
 * recommendations honour them on the next pick, on every device.
 */
export function useRadioBans() {
  const queryClient = useQueryClient();
  const session = useSession();
  const online = session.phase === "ready";
  const query = useQuery({
    queryKey: radioBansKey,
    queryFn: getRadioBans,
    enabled: online,
    staleTime: 60_000,
    retry: false,
  });
  const bans = query.data;
  const index = useMemo(
    () => (bans ? radioBanIndex(bans) : emptyRadioBanIndex),
    [bans],
  );
  const mutation = useMutation({
    mutationFn: async ({
      target,
      banned,
    }: {
      target: BanTarget;
      banned: boolean;
    }) => {
      if (banned) await banFromRadio(target);
      else await unbanFromRadio(target);
    },
    onMutate: async ({ target, banned }) => {
      await queryClient.cancelQueries({ queryKey: radioBansKey });
      const previous = queryClient.getQueryData<RadioBan[]>(radioBansKey);
      queryClient.setQueryData<RadioBan[]>(radioBansKey, (current) =>
        applyRadioBanChange(current ?? [], target, banned),
      );
      return { previous };
    },
    onError: (_error, _change, context) => {
      queryClient.setQueryData(radioBansKey, context?.previous);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: radioBansKey });
    },
  });
  const { mutateAsync } = mutation;
  const isBanned = useCallback(
    (target: RadioBanTarget) => isRadioBanned(index, target),
    [index],
  );
  return {
    bans: bans ?? [],
    /** True once the server's list has arrived. */
    loaded: Boolean(bans),
    index,
    online,
    loading: online && query.isPending,
    error: query.error,
    pending: mutation.isPending,
    refetch: query.refetch,
    isBanned,
    excludes: useCallback(
      (song: BannableSong) => isSongExcludedByBans(index, song),
      [index],
    ),
    setBanned: useCallback(
      (target: BanTarget, banned: boolean) =>
        mutateAsync({ target, banned }).then(
          () => true,
          () => false,
        ),
      [mutateAsync],
    ),
  };
}

/** "Ban this album" / "Unban this album" in an action drawer. */
export function BanAction({
  onDone,
  target,
}: {
  onDone?: () => void;
  target: BanTarget;
}) {
  const bans = useRadioBans();
  const [failed, setFailed] = useState(false);
  if (!bans.online) return null;
  const banned = bans.isBanned(target);
  return (
    <DrawerAction
      icon={Ban}
      label={
        failed
          ? `Could not ${banned ? "unban" : "ban"} · Try again`
          : radioBanActionLabel(target.kind, banned)
      }
      onPress={() => {
        setFailed(false);
        void bans.setBanned(target, !banned).then((saved) => {
          if (saved) onDone?.();
          else setFailed(true);
        });
      }}
    />
  );
}

/** Ban icon for banned items; renders nothing otherwise. */
export function BannedMark({
  size = 14,
  target,
}: {
  size?: number;
  target: RadioBanTarget;
}) {
  const { isBanned } = useRadioBans();
  if (!isBanned(target)) return null;
  return (
    <Ban
      accessibilityLabel={`Banned ${radioBanKindLabel(target.kind)}`}
      color={palette.danger}
      size={size}
    />
  );
}
