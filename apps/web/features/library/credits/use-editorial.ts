"use client";

import {
  focusQueryString,
  getAlbumAbout,
  getArtistAbout,
  getArtistCredits,
  getArtistDiscography,
  isApiError,
  saveNote,
  type FocusSelection,
} from "@parson/music-sdk";
import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { toast } from "sonner";

const noRetryOnMissing = (failureCount: number, error: unknown) =>
  !(isApiError(error) && error.response?.status === 404) && failureCount < 2;

export function useAlbumAbout(id: string | null | undefined) {
  return useQuery({
    queryKey: ["album-about", id],
    queryFn: () => getAlbumAbout(id!),
    enabled: Boolean(id),
    retry: noRetryOnMissing,
    staleTime: 60_000,
  });
}

export function useArtistAbout(id: string | null | undefined) {
  return useQuery({
    queryKey: ["artist-about", id],
    queryFn: () => getArtistAbout(id!),
    enabled: Boolean(id),
    retry: noRetryOnMissing,
    staleTime: 60_000,
  });
}

export function useArtistCredits(
  id: string | null | undefined,
  enabled = true,
) {
  return useQuery({
    queryKey: ["artist-credits", id],
    queryFn: () => getArtistCredits(id!),
    enabled: Boolean(id) && enabled,
    retry: noRetryOnMissing,
    staleTime: 60_000,
  });
}

/** A person's discography narrowed by a Focus selection; the previous
 * result stays on screen while a new selection loads. */
export function useArtistDiscography(
  id: string | null | undefined,
  selection: FocusSelection,
  enabled = true,
) {
  return useQuery({
    queryKey: ["artist-discography", id, focusQueryString(selection)],
    queryFn: () => getArtistDiscography(id!, selection),
    enabled: Boolean(id) && enabled,
    retry: noRetryOnMissing,
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  });
}

export function useSaveNote(entityType: "album" | "artist", id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: string) => saveNote(entityType, id, body),
    onSuccess: async (note) => {
      toast.success(note ? "Note saved." : "Note removed.");
      await queryClient.invalidateQueries({
        queryKey: [`${entityType}-about`, id],
      });
    },
    onError: () => toast("Could not save your note."),
  });
}
