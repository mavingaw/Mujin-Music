"use client";

import { usePlayer } from "@/features/player/player-context";
import type { QueueInput } from "@/features/player/player-model";
import { getFocusPage, withEntity, type FocusQuery } from "@parson/music-sdk";
import { useCallback } from "react";
import { toast } from "sonner";
import { catalogSongsToQueue, shuffled } from "./focus-state";

const MAX_PLAY_SONGS = 500;
const PAGE = 200;

/** Loads up to 500 songs of a focus, in its order, as queue entries. */
export async function loadFocusQueue(query: FocusQuery): Promise<QueueInput[]> {
  const songsQuery = withEntity(query, "songs");
  const page = await getFocusPage(songsQuery, 0, PAGE);
  let songs = page.songs;
  for (
    let offset = songs.length;
    offset < Math.min(page.total, MAX_PLAY_SONGS);
    offset += PAGE
  ) {
    const next = await getFocusPage(songsQuery, offset, PAGE);
    if (!next.songs.length) break;
    songs = songs.concat(next.songs);
  }
  return catalogSongsToQueue(songs.slice(0, MAX_PLAY_SONGS));
}

/** Plays queue entries from the first one, optionally shuffled. */
export function usePlayQueue() {
  const player = usePlayer();
  return useCallback(
    (queue: QueueInput[], shuffle: boolean) => {
      const ordered = shuffle ? shuffled(queue) : queue;
      const first = ordered[0];
      if (!first) {
        toast("Nothing to play in this focus.");
        return;
      }
      player.setQueue(ordered);
      player.setCurrentSongIndex(0);
      player.setSongCallback(first.song, first.artist, first.album);
      player.playAudioSource();
    },
    [player],
  );
}

/** Plays (or shuffles) every song a focus matches. */
export function usePlayFocus() {
  const playQueue = usePlayQueue();
  return useCallback(
    async (query: FocusQuery, shuffle: boolean) => {
      try {
        playQueue(await loadFocusQueue(query), shuffle);
      } catch {
        toast("Could not play this focus.");
      }
    },
    [playQueue],
  );
}
