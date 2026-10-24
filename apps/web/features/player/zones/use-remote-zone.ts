"use client";

import {
  estimateZonePosition,
  getSongInfo,
  getSongInfos,
  type Zone,
} from "@parson/music-sdk";
import type { LibrarySong } from "@parson/music-sdk/types";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { QueueItem } from "../player-model";

/** Ticking position estimate for a remote zone, in seconds. */
export function useZoneClock(zone: Zone | null, fetchedAt: number) {
  const [now, setNow] = useState(() => Date.now());
  const playing = zone?.status === "playing";
  useEffect(() => {
    if (!playing) return;
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [playing]);
  if (!zone) return 0;
  return estimateZonePosition(zone, fetchedAt, Math.max(now, fetchedAt)) / 1000;
}

/** Full song info for what a remote zone plays (for credits and links). */
export function useZoneSong(zone: Zone | null): LibrarySong | null {
  const songId = zone?.now_playing?.song_id ?? null;
  return (
    useQuery({
      queryKey: ["zone-song", songId],
      queryFn: () => getSongInfo(songId!, false) as Promise<LibrarySong>,
      enabled: Boolean(songId),
      staleTime: 5 * 60_000,
    }).data ?? null
  );
}

/** The remote zone's queue as player queue items. */
export function useZoneQueue(zone: Zone | null): QueueItem[] {
  const ids = zone?.queue.song_ids ?? [];
  const key = ids.join(",");
  const infos = useQuery({
    queryKey: ["zone-queue", key],
    queryFn: () => getSongInfos(ids, false),
    enabled: ids.length > 0,
    staleTime: 5 * 60_000,
  }).data;
  return useMemo(() => {
    if (!infos) return [];
    return key
      .split(",")
      .map((id) => infos[id] as LibrarySong | undefined)
      .filter((song): song is LibrarySong => Boolean(song?.album_object))
      .map((song) => ({
        song,
        album: song.album_object,
        artist: song.artist_object,
        origin: "manual" as const,
        queuePosition: null,
      }));
  }, [infos, key]); // `key` captures `ids`.
}
