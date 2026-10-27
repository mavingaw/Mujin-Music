"use client";

import { playOnZoneCommand, queueOnZoneCommand } from "@parson/music-sdk";
import { useCallback, useMemo } from "react";
import { toast } from "sonner";
import { usePlayer } from "../player-api";
import type { QueueInput } from "../player-model";
import {
  queuedOnZoneMessage,
  resolvePlaybackTarget,
  zoneActionError,
  type PlaybackTarget,
} from "./zone-playback-model";
import { useOptionalZones } from "./zones-provider";

export type ZonePlayback = {
  target: PlaybackTarget;
  /** The song the selected zone is on (this device's when local). */
  currentSongId: string | null;
  currentAlbumId: string | null;
  isPlaying: boolean;
  /** Replaces the selected zone's queue with `items` and plays `index`. */
  playQueue: (items: QueueInput[], index?: number) => Promise<void>;
  /** "Play next"; `label` names what was added in the confirmation. */
  playNext: (items: QueueInput[], label?: string) => Promise<void>;
  /** "Add to queue". */
  addToQueue: (items: QueueInput[], label?: string) => Promise<void>;
  togglePlayPause: () => Promise<void>;
};

function songIds(items: QueueInput[]): string[] {
  return items
    .map((item) => item.song.id)
    .filter((id): id is string => Boolean(id));
}

/**
 * Every library Play, Play next and Add to queue goes through here, so it
 * reaches the zone chosen in the zone picker: a remote zone (a Sonos room,
 * a bridge DAC, another Parson app) gets the server's load or queue command
 * and the browser stays silent; only "This device" plays in the browser.
 */
export function useZonePlayback(): ZonePlayback {
  const player = usePlayer();
  const zones = useOptionalZones();
  const target = useMemo(
    () =>
      zones
        ? resolvePlaybackTarget({
            requestedZoneId: zones.requestedZoneId,
            localZoneId: zones.localZoneId,
            zones: zones.zones,
            loaded: zones.loaded,
          })
        : ({ kind: "local" } as const),
    [zones],
  );
  const remoteZone = zones?.remoteZone ?? null;
  const send = zones?.send;

  const playQueue = useCallback(
    async (items: QueueInput[], index = 0) => {
      if (!items.length) return;
      const start = Math.min(Math.max(index, 0), items.length - 1);
      if (target.kind === "zone" && send) {
        if (!target.canLoad) {
          toast(
            zoneActionError(
              new Error("zone_cannot_receive_transfer"),
              target.name,
            ),
          );
          return;
        }
        try {
          await send(target.zoneId, playOnZoneCommand(songIds(items), start));
        } catch (error) {
          toast(zoneActionError(error, target.name));
        }
        return;
      }
      const item = items[start]!;
      player.setQueue(items);
      player.setCurrentSongIndex(start);
      player.setSongCallback(item.song, item.artist, item.album);
      player.playAudioSource();
    },
    [player, send, target],
  );

  const enqueue = useCallback(
    async (items: QueueInput[], next: boolean, label?: string) => {
      if (!items.length) return;
      const what =
        label ??
        (items.length === 1
          ? (items[0]!.song.name ?? "Song")
          : `${items.length} songs`);
      if (target.kind === "zone" && send) {
        if (!target.canQueue) {
          toast(
            zoneActionError(
              new Error("edit_the_cast_queue_on_the_phone"),
              target.name,
            ),
          );
          return;
        }
        try {
          await send(target.zoneId, queueOnZoneCommand(songIds(items), next));
          toast.success(queuedOnZoneMessage(what, target.name, next));
        } catch (error) {
          toast(zoneActionError(error, target.name));
        }
        return;
      }
      if (next) {
        player.addNextToQueue(items);
        toast.success(`${what} will play next`);
      } else {
        player.addToQueue(items);
        toast.success(`Added ${what} to queue`);
      }
    },
    [player, send, target],
  );

  const playNext = useCallback(
    (items: QueueInput[], label?: string) => enqueue(items, true, label),
    [enqueue],
  );
  const addToQueue = useCallback(
    (items: QueueInput[], label?: string) => enqueue(items, false, label),
    [enqueue],
  );

  const togglePlayPause = useCallback(async () => {
    if (target.kind === "zone" && send) {
      try {
        await send(target.zoneId, { command: "play_pause" });
      } catch (error) {
        toast(zoneActionError(error, target.name));
      }
      return;
    }
    player.togglePlayPause();
  }, [player, send, target]);

  const remote = target.kind === "zone";
  return {
    target,
    currentSongId: remote
      ? (remoteZone?.now_playing?.song_id ?? null)
      : player.song.id || null,
    currentAlbumId: remote
      ? (remoteZone?.now_playing?.album_id ?? null)
      : (player.song.album_object?.id ?? player.album.id) || null,
    isPlaying: remote ? remoteZone?.status === "playing" : player.isPlaying,
    playQueue,
    playNext,
    addToQueue,
    togglePlayPause,
  };
}
