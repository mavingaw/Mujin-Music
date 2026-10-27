"use client";

import {
  addLiveStation,
  getLiveFacets,
  getLiveNowPlaying,
  getLiveRadioHome,
  getLiveStation,
  getMyLiveStations,
  isApiError,
  LIVE_TITLE_POLL_MS,
  liveStationIdFromSongId,
  removeLiveStation,
  reorderLiveStations,
  saveLiveStation,
  searchLiveStations,
  verifyLiveStation,
  type LiveFacetKind,
  type LiveSearch,
  type LiveStation,
  type LiveStationDetail,
  type NewLiveStation,
} from "@parson/music-sdk";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "sonner";
import getBaseURL from "@/lib/api/server-url";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { browserRegion, stationQueueItem } from "./live-radio-model";

const keys = {
  home: (country: string | null) => ["live-radio", "home", country] as const,
  search: (search: LiveSearch) => ["live-radio", "search", search] as const,
  station: (id: string) => ["live-radio", "station", id] as const,
  my: ["live-radio", "my"] as const,
  facets: (kind: LiveFacetKind) => ["live-radio", "facets", kind] as const,
  playing: (id: string) => ["live-radio", "now-playing", id] as const,
};

/** The server's message for a failed radio request. */
export function liveRadioError(error: unknown, fallback: string): string {
  if (isApiError(error)) {
    const message = (error.response?.data as { message?: string } | undefined)
      ?.message;
    if (message) return message;
  }
  return fallback;
}

export function useLiveRadioHome() {
  const country =
    typeof navigator === "undefined"
      ? null
      : browserRegion(navigator.languages);
  return useQuery({
    queryKey: keys.home(country),
    queryFn: () => getLiveRadioHome(country),
    staleTime: 5 * 60_000,
  });
}

const PAGE = 48;

export function useLiveSearch(search: LiveSearch, enabled = true) {
  return useInfiniteQuery({
    queryKey: keys.search(search),
    queryFn: ({ pageParam }) =>
      searchLiveStations({ ...search, offset: pageParam, limit: PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last) =>
      last.has_more ? last.offset + last.items.length : undefined,
    enabled,
    staleTime: 5 * 60_000,
  });
}

export function useLiveStation(id: string | null) {
  return useQuery({
    queryKey: keys.station(id ?? ""),
    queryFn: () => getLiveStation(id!),
    enabled: Boolean(id),
    staleTime: 60_000,
  });
}

/**
 * Has the server read a station's real format from its audio, once per
 * station per visit, and folds the answer into the station page.
 */
export function useVerifyLiveStation(station: LiveStation | undefined) {
  const client = useQueryClient();
  const id = station?.id ?? "";
  return useQuery({
    queryKey: ["live-radio", "verify", id],
    queryFn: async () => {
      const verified = await verifyLiveStation(id);
      client.setQueryData<LiveStationDetail>(keys.station(id), (detail) =>
        detail ? { ...detail, station: verified } : detail,
      );
      return verified;
    },
    enabled: Boolean(station) && !station?.quality.verified,
    retry: false,
    staleTime: 30 * 60_000,
  });
}

export function useMyLiveStations() {
  return useQuery({
    queryKey: keys.my,
    queryFn: getMyLiveStations,
    staleTime: 60_000,
  });
}

export function useLiveFacets(kind: LiveFacetKind, enabled = true) {
  return useQuery({
    queryKey: keys.facets(kind),
    queryFn: () => getLiveFacets(kind, 300),
    enabled,
    staleTime: 12 * 60 * 60_000,
  });
}

/**
 * What a station is playing, polled while `active` (it is playing here or
 * on a zone). Titles come from the stream's own ICY or HLS metadata.
 */
export function useLiveNowPlaying(
  songOrStationId: string | null,
  active = true,
) {
  const stationId =
    liveStationIdFromSongId(songOrStationId) ??
    (songOrStationId && !songOrStationId.includes(":")
      ? songOrStationId
      : null);
  return useQuery({
    queryKey: keys.playing(stationId ?? ""),
    queryFn: async () =>
      (await getLiveNowPlaying([stationId!]))[stationId!] ?? null,
    enabled: Boolean(stationId),
    refetchInterval: active ? LIVE_TITLE_POLL_MS : false,
    staleTime: 5_000,
  });
}

/** Saves to or removes from My Live Radio, updating every list. */
export function useSaveLiveStation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async ({
      station,
      saved,
    }: {
      station: LiveStation;
      saved: boolean;
    }) => {
      if (saved) {
        await removeLiveStation(station.id);
        return { ...station, saved: false };
      }
      return saveLiveStation(station.id);
    },
    onSuccess: (station) => {
      void client.invalidateQueries({ queryKey: ["live-radio"] });
      toast.success(
        station.saved
          ? `${station.name} added to My Live Radio`
          : `${station.name} removed from My Live Radio`,
      );
    },
    onError: (error) =>
      toast.error(liveRadioError(error, "My Live Radio could not be updated.")),
  });
}

export function useAddLiveStation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (station: NewLiveStation) => addLiveStation(station),
    onSuccess: ({ station }) => {
      void client.invalidateQueries({ queryKey: ["live-radio"] });
      toast.success(`${station.name} added to My Live Radio`);
    },
  });
}

export function useReorderLiveStations() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (ids: string[]) => reorderLiveStations(ids),
    onMutate: (ids) => {
      const previous = client.getQueryData<LiveStation[]>(keys.my);
      if (previous) {
        const byId = new Map(previous.map((station) => [station.id, station]));
        client.setQueryData(
          keys.my,
          ids.map((id) => byId.get(id)).filter(Boolean),
        );
      }
      return { previous };
    },
    onError: (_error, _ids, context) => {
      if (context?.previous) client.setQueryData(keys.my, context.previous);
      toast.error("The new order could not be saved.");
    },
  });
}

/** Plays stations on the zone chosen in the zone picker. */
export function usePlayStation() {
  const playback = useZonePlayback();
  const play = useCallback(
    (station: LiveStation) => {
      if (playback.currentSongId === station.song_id) {
        void playback.togglePlayPause();
        return;
      }
      void playback.playQueue([stationQueueItem(station, getBaseURL())], 0);
    },
    [playback],
  );
  const isCurrent = (station: LiveStation) =>
    playback.currentSongId === station.song_id;
  return {
    play,
    isCurrent,
    isPlaying: (station: LiveStation) =>
      isCurrent(station) && playback.isPlaying,
    target: playback.target,
  };
}
