"use client";

import {
  getAlbumInfo,
  getArtistInfo,
  type LibraryAlbum,
} from "@parson/music-sdk";
import { useCallback } from "react";
import { toast } from "sonner";
import { albumQueue, artistQueue } from "./play-queue-order";
import type { QueueInput } from "./player-model";
import { useStartRadio } from "./radio/radio-controls";
import { useZonePlayback } from "./zones/use-zone-playback";

/** What a Play-now split menu plays. */
export type PlayTarget =
  | { kind: "album"; id: string; name?: string }
  | { kind: "artist"; id: string; name?: string }
  | { kind: "track"; id: string; name?: string; item: QueueInput };

/** The four choices of a Play-now split menu, as Roon offers them. */
export type PlayAction = "play" | "next" | "queue" | "radio";

export const PLAY_ACTIONS: readonly { action: PlayAction; label: string }[] = [
  { action: "play", label: "Play now" },
  { action: "next", label: "Add next" },
  { action: "queue", label: "Add to queue" },
  { action: "radio", label: "Start radio" },
];

/** Runs a Play-now split choice in the selected zone. */
export function usePlayActions() {
  const playback = useZonePlayback();
  const startRadio = useStartRadio();
  return useCallback(
    async (target: PlayTarget, action: PlayAction) => {
      if (action === "radio") {
        await startRadio({
          kind: target.kind === "track" ? "song" : target.kind,
          id: target.id,
        });
        return;
      }
      let items: QueueInput[];
      let label = target.name;
      try {
        if (target.kind === "track") {
          items = [target.item];
        } else if (target.kind === "album") {
          const album = (await getAlbumInfo(target.id, false)) as LibraryAlbum;
          items = albumQueue(album);
          label ??= album.name;
        } else {
          const artist = await getArtistInfo(target.id);
          items = artistQueue(artist);
          label ??= artist.name;
        }
      } catch {
        toast(`Could not load this ${target.kind}.`);
        return;
      }
      if (!items.length) {
        toast("Nothing here to play.");
        return;
      }
      if (action === "play") await playback.playQueue(items);
      else if (action === "next") await playback.playNext(items, label);
      else await playback.addToQueue(items, label);
    },
    [playback, startRadio],
  );
}
