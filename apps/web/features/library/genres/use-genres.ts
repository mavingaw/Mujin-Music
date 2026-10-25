"use client";

import {
  getGenrePage,
  getGenreTree,
  isApiError,
  resetAlbumGenres,
  setAlbumGenres,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

const noRetryOnMissing = (failureCount: number, error: unknown) =>
  !(isApiError(error) && error.response?.status === 404) && failureCount < 2;

export function useGenreTree(enabled = true) {
  return useQuery({
    queryKey: ["genre-tree"],
    queryFn: getGenreTree,
    enabled,
    staleTime: 60_000,
  });
}

export function useGenrePage(slug: string | null | undefined) {
  return useQuery({
    queryKey: ["genre-page", slug],
    queryFn: () => getGenrePage(slug!),
    enabled: Boolean(slug),
    retry: noRetryOnMissing,
    staleTime: 60_000,
    // Poll while the description is still being looked up.
    refetchInterval: (query) =>
      query.state.data?.enrichment.task?.status === "pending" ? 8_000 : false,
  });
}

/** Administrator: saves an album's genres, or (`null`) resets them. */
export function useSaveAlbumGenres(albumId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (genres: string[] | null) =>
      genres ? setAlbumGenres(albumId, genres) : resetAlbumGenres(albumId),
    onSuccess: async (_, genres) => {
      toast.success(genres ? "Genres saved." : "Genres reset.");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["album-about", albumId] }),
        queryClient.invalidateQueries({ queryKey: ["genre-tree"] }),
        queryClient.invalidateQueries({ queryKey: ["genre-page"] }),
        queryClient.invalidateQueries({ queryKey: ["artist-about"] }),
      ]);
    },
    onError: () => toast("Could not save the genres."),
  });
}
