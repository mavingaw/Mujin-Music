"use client";

import {
  addFavoriteSong,
  heartState,
  isFavoriteSong,
  nextHeartState,
  removeFavoriteSong,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban, Heart, Loader2 } from "lucide-react";
import { useRadioBans } from "./radio-bans";

const favoriteSongDetailsKey = ["favorite-song-details"] as const;

export function useFavoriteSong(songId: string, enabled = true) {
  const queryClient = useQueryClient();
  const membershipKey = ["favorite-membership", songId] as const;
  const membership = useQuery({
    queryKey: membershipKey,
    queryFn: () => isFavoriteSong(songId),
    enabled,
    retry: false,
    staleTime: 30_000,
  });
  const liked = membership.data ?? false;
  const mutation = useMutation({
    mutationFn: async (nextLiked: boolean) => {
      if (nextLiked) await addFavoriteSong(songId);
      else await removeFavoriteSong(songId);
      return nextLiked;
    },
    onMutate: async (nextLiked) => {
      await queryClient.cancelQueries({ queryKey: membershipKey });
      const previous = queryClient.getQueryData<boolean>(membershipKey);
      queryClient.setQueryData(membershipKey, nextLiked);
      return { previous };
    },
    onError: (_error, _nextLiked, context) => {
      if (context?.previous !== undefined) {
        queryClient.setQueryData(membershipKey, context.previous);
      }
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: membershipKey });
      void queryClient.invalidateQueries({
        queryKey: favoriteSongDetailsKey,
      });
    },
  });

  return {
    error: membership.error ?? mutation.error,
    liked,
    checking: enabled && membership.isPending,
    loading: mutation.isPending,
    toggle: () => mutation.mutate(!liked),
  };
}

/**
 * The song heart. Like Roon's, it has three states: not a favourite,
 * favourite (Liked Songs), and banned (Radio and Shuffle never pick it).
 */
export default function FavoriteButton({
  className = "",
  songId,
  songName,
}: {
  className?: string;
  songId: string;
  songName?: string;
}) {
  const favorite = useFavoriteSong(songId);
  const bans = useRadioBans();
  const banned = bans.isBanned({ kind: "song", id: songId });
  const state = heartState(favorite.liked, banned);
  const target = songName ? ` ${songName}` : "";
  const next = nextHeartState(state);
  const label =
    state === "banned"
      ? `${songName ?? "This song"} is banned. Unban`
      : state === "liked"
        ? `Remove${target} from Liked Songs and ban it from Radio`
        : `Add${target} to Liked Songs`;

  const cycle = () => {
    const ban = { kind: "song" as const, id: songId, name: songName };
    if (next === "liked") {
      favorite.toggle();
    } else if (next === "banned") {
      favorite.toggle();
      bans.setBanned(ban, true);
    } else {
      bans.setBanned(ban, false);
    }
  };

  return (
    <button
      aria-label={label}
      aria-pressed={state !== "none"}
      className={`relative z-10 flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-colors ${
        state === "liked"
          ? "text-rose-400 hover:bg-rose-400/10"
          : state === "banned"
            ? "text-rose-400/90 hover:bg-rose-400/10"
            : "text-zinc-500 hover:bg-white/10 hover:text-white"
      } ${className}`}
      data-heart-state={state}
      disabled={favorite.loading || bans.pending}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        cycle();
      }}
      title={
        favorite.error
          ? "Could not update Liked Songs. Try again."
          : state === "banned"
            ? "Banned: Radio and Shuffle skip it. Click to unban."
            : state === "liked"
              ? "In Liked Songs. Click again to ban."
              : "Add to Liked Songs"
      }
      type="button"
    >
      {favorite.loading ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : state === "banned" ? (
        <Ban className="h-4 w-4" />
      ) : (
        <Heart
          className={`h-4 w-4 ${favorite.liked ? "fill-current" : ""} ${favorite.checking ? "opacity-60" : ""}`}
        />
      )}
    </button>
  );
}
