"use client";

import {
  clearAlbumIdentification,
  getArtistWorks,
  getSongWorks,
  getWork,
  identifyAlbum,
  isApiError,
  searchAlbumReleases,
  type ReleaseMatch,
  type ReleaseSearchTerms,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

const noRetryOnMissing = (failureCount: number, error: unknown) =>
  !(isApiError(error) && error.response?.status === 404) && failureCount < 2;

export function useWork(mbid: string | null | undefined) {
  return useQuery({
    queryKey: ["work", mbid],
    queryFn: () => getWork(mbid!),
    enabled: Boolean(mbid),
    retry: noRetryOnMissing,
    staleTime: 60_000,
  });
}

export function useArtistWorks(id: string | null | undefined, enabled = true) {
  return useQuery({
    queryKey: ["artist-works", id],
    queryFn: () => getArtistWorks(id!),
    enabled: Boolean(id) && enabled,
    retry: noRetryOnMissing,
    staleTime: 60_000,
  });
}

export function useSongWorks(id: string | null | undefined, enabled = true) {
  return useQuery({
    queryKey: ["song-works", id],
    queryFn: () => getSongWorks(id!),
    enabled: Boolean(id) && enabled,
    retry: noRetryOnMissing,
    staleTime: 5 * 60_000,
  });
}

/** The status and message of a failed identify search. */
export function identifyError(error: unknown) {
  if (isApiError(error) && error.response?.status === 409) {
    return {
      disabled: true,
      message:
        "Searching MusicBrainz needs online metadata. Turn it on in Settings > Library.",
    };
  }
  return {
    disabled: false,
    message: "MusicBrainz did not answer. Try again in a moment.",
  };
}

export function useReleaseSearch(
  albumId: string,
  terms: ReleaseSearchTerms,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["identify-album", albumId, terms],
    queryFn: () => searchAlbumReleases(albumId, terms),
    enabled,
    retry: false,
    staleTime: 5 * 60_000,
  });
}

export function useIdentifyAlbum(albumId: string) {
  const queryClient = useQueryClient();
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: ["album-about", albumId] });
    await queryClient.invalidateQueries({
      queryKey: ["identify-album", albumId],
    });
  };
  const choose = useMutation({
    mutationFn: (release: ReleaseMatch) => identifyAlbum(albumId, release),
    onSuccess: async () => {
      toast.success(
        "Album identified. Credits, works and reviews will update shortly.",
      );
      await refresh();
    },
    onError: () => toast("Could not save the match."),
  });
  const clear = useMutation({
    mutationFn: () => clearAlbumIdentification(albumId),
    onSuccess: async () => {
      toast.success("Manual match removed.");
      await refresh();
    },
    onError: () => toast("Could not remove the match."),
  });
  return { choose, clear };
}
