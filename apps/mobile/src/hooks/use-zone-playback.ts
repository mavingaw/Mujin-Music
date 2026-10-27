import {
  playOnZoneCommand,
  queueOnZoneCommand,
  type ZoneCommand,
} from "@parson/music-sdk";
import type { LibrarySong } from "@parson/music-sdk/types";
import { useCallback, useMemo } from "react";
import { Alert } from "react-native";

import { mobilePlaybackTarget } from "@/lib/zones";
import { shuffledQueue } from "@/lib/shuffle-queue";
import { usePlayer } from "@/providers/player-provider";
import { useZones } from "@/providers/zones-provider";

function zoneError(error: unknown, zoneName: string) {
  const code = (error as { response?: { data?: { error?: string } } })?.response
    ?.data?.error;
  Alert.alert(
    zoneName,
    code ? code.replaceAll("_", " ") : `${zoneName} did not take that.`,
  );
}

/**
 * Library Play, Shuffle, Play next and Add to queue, sent to the zone
 * picked in the zone sheet: a remote zone (Sonos room, bridge DAC, another
 * Parson app) gets the server command and the phone stays silent; with this
 * phone picked, it plays here.
 */
export function useZonePlayback() {
  const player = usePlayer();
  const zones = useZones();
  const target = useMemo(
    () =>
      mobilePlaybackTarget(
        zones.requestedZoneId,
        zones.localZoneId,
        zones.zones,
        zones.loaded,
      ),
    [zones.loaded, zones.localZoneId, zones.requestedZoneId, zones.zones],
  );
  const { send } = zones;

  const remote = useCallback(
    async (command: ZoneCommand, allowed: boolean) => {
      if (target.kind !== "zone") return false;
      if (!allowed) {
        Alert.alert(
          target.name,
          "This zone plays what the device that started it sends.",
        );
        return true;
      }
      try {
        await send(target.zoneId, command);
      } catch (error) {
        zoneError(error, target.name);
      }
      return true;
    },
    [send, target],
  );

  const playSong = useCallback(
    (song: LibrarySong, queue?: LibrarySong[]) => {
      const songs = queue?.length ? queue : [song];
      const index = Math.max(
        0,
        songs.findIndex((candidate) => candidate.id === song.id),
      );
      if (target.kind === "zone") {
        void remote(
          playOnZoneCommand(
            songs.map((candidate) => candidate.id),
            index,
          ),
          target.canLoad,
        );
        return;
      }
      player.playSong(song, queue);
    },
    [player, remote, target],
  );

  const playShuffled = useCallback(
    (songs: LibrarySong[]) => {
      if (target.kind === "zone") {
        if (!songs.length) return;
        const shuffled = shuffledQueue(
          songs,
          Math.floor(Math.random() * songs.length),
        ).queue;
        void remote(
          playOnZoneCommand(shuffled.map((song) => song.id)),
          target.canLoad,
        );
        return;
      }
      player.playShuffled(songs);
    },
    [player, remote, target],
  );

  const addToQueue = useCallback(
    (songs: LibrarySong[]) => {
      if (target.kind === "zone") {
        if (!songs.length) return;
        void remote(
          queueOnZoneCommand(
            songs.map((song) => song.id),
            false,
          ),
          target.canQueue,
        );
        return;
      }
      player.addToQueue(songs);
    },
    [player, remote, target],
  );

  const addNext = useCallback(
    (song: LibrarySong) => {
      if (target.kind === "zone") {
        void remote(queueOnZoneCommand([song.id], true), target.canQueue);
        return;
      }
      player.addNext(song);
    },
    [player, remote, target],
  );

  return { target, playSong, playShuffled, addToQueue, addNext };
}
