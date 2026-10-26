"use client";

import { ContextMenuItem } from "@/components/ui/context-menu";
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
  type RadioBanIndex,
  type RadioBanTarget,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban } from "lucide-react";
import { useCallback, useMemo } from "react";
import { toast } from "sonner";

export const radioBansKey = ["radio-bans"] as const;

export type BanTarget = RadioBanTarget & { name?: string | null };

type BanChange = { target: BanTarget; banned: boolean; quiet?: boolean };

/**
 * The listener's Radio bans, shared by menus, hearts, badges, the player and
 * the Library's Banned view. Changes apply optimistically and reach the
 * server's Radio and Shuffle on the very next pick.
 */
export function useRadioBans() {
  const queryClient = useQueryClient();
  const { query, index, isBanned, excludes } = useRadioBanIndex();
  const bans = query.data;

  const mutation = useMutation({
    mutationFn: async ({ target, banned }: BanChange) => {
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
    onError: (_error, { banned }, context) => {
      queryClient.setQueryData(radioBansKey, context?.previous);
      toast(banned ? "Could not ban this." : "Could not unban this.");
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: radioBansKey });
    },
  });
  const { mutate } = mutation;

  const setBanned = useCallback(
    (target: BanTarget, banned: boolean, options?: { quiet?: boolean }) => {
      mutate(
        { target, banned },
        {
          onSuccess: () => {
            if (options?.quiet) return;
            const what = target.name
              ? `“${target.name}”`
              : `This ${radioBanKindLabel(target.kind)}`;
            const undo = {
              label: "Undo",
              onClick: () => mutate({ target, banned: !banned }),
            };
            if (banned) {
              toast.success(`${what} is banned`, {
                description: "Radio, Shuffle and recommendations skip it.",
                action: undo,
              });
            } else {
              toast.success(`${what} is no longer banned`, { action: undo });
            }
          },
        },
      );
    },
    [mutate],
  );

  return {
    bans: bans ?? [],
    index,
    loading: query.isPending,
    error: query.error,
    pending: mutation.isPending,
    isBanned,
    excludes,
    setBanned,
    toggle: useCallback(
      (target: BanTarget) => setBanned(target, !isRadioBanned(index, target)),
      [index, setBanned],
    ),
  };
}

/** Read-only view of the bans, for badges and filters. */
export function useRadioBanIndex() {
  const query = useQuery({
    queryKey: radioBansKey,
    queryFn: getRadioBans,
    staleTime: 60_000,
    retry: false,
  });
  const bans = query.data;
  const index: RadioBanIndex = useMemo(
    () => (bans ? radioBanIndex(bans) : emptyRadioBanIndex),
    [bans],
  );
  const isBanned = useCallback(
    (target: RadioBanTarget) => isRadioBanned(index, target),
    [index],
  );
  const excludes = useCallback(
    (song: BannableSong) => isSongExcludedByBans(index, song),
    [index],
  );
  return { query, index, isBanned, excludes };
}

/** "Ban this artist" / "Unban this artist" for a context menu. */
export function BanMenuItem({ target }: { target: BanTarget }) {
  const bans = useRadioBans();
  const banned = bans.isBanned(target);
  return (
    <ContextMenuItem
      data-variant={banned ? undefined : "destructive"}
      onSelect={() => bans.setBanned(target, !banned)}
    >
      <Ban className="h-4 w-4" />
      {radioBanActionLabel(target.kind, banned)}
    </ContextMenuItem>
  );
}

/**
 * Small ban icon shown next to banned items, as Roon does. Renders nothing
 * for items that are not banned.
 */
export function BannedBadge({
  className = "",
  target,
}: {
  className?: string;
  target: RadioBanTarget;
}) {
  const { isBanned } = useRadioBanIndex();
  if (!isBanned(target)) return null;
  const label = `Banned ${radioBanKindLabel(target.kind)}: Radio and Shuffle skip it`;
  return (
    <span
      aria-label={label}
      className={`inline-flex shrink-0 items-center text-rose-400 ${className}`}
      role="img"
      title={label}
    >
      <Ban aria-hidden="true" className="h-3.5 w-3.5" />
    </span>
  );
}

/** Pill toggle for album and artist pages: "Ban" / "Banned". */
export function BanToggleButton({ target }: { target: BanTarget }) {
  const bans = useRadioBans();
  const banned = bans.isBanned(target);
  const kind = radioBanKindLabel(target.kind);
  return (
    <button
      aria-label={
        banned
          ? `Unban this ${kind}`
          : `Ban this ${kind} from Radio and Shuffle`
      }
      aria-pressed={banned}
      className={`inline-flex h-10 items-center gap-2 rounded-full border px-4 text-sm font-semibold transition-colors ${
        banned
          ? "border-rose-400/50 bg-rose-500/10 text-rose-200 hover:bg-rose-500/20"
          : "border-white/15 text-zinc-300 hover:border-white/35 hover:bg-white/6 hover:text-white"
      }`}
      onClick={() => bans.setBanned(target, !banned)}
      title={
        banned
          ? "Banned: Radio, Shuffle and recommendations skip it. Click to unban."
          : "Never play in Radio, Shuffle or recommendations"
      }
      type="button"
    >
      <Ban aria-hidden="true" className="h-4 w-4" />
      {banned ? "Banned" : "Ban"}
    </button>
  );
}
