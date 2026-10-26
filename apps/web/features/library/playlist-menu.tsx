"use client";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { getPlaylist } from "@parson/music-sdk";
import { ListEnd, ListMusic, ListStart, Play } from "lucide-react";
import Link from "next/link";
import { toast } from "sonner";

export default function PlaylistMenu({
  children,
  playlistId,
}: {
  children: React.ReactNode;
  playlistId: number;
}) {
  const playback = useZonePlayback();

  const run = async (action: "play" | "next" | "end") => {
    try {
      const playlist = await getPlaylist(playlistId);
      const items = playlist.songs.map((song) => ({
        song,
        artist: song.artist_object,
        album: song.album_object,
      }));
      if (!items.length) {
        return;
      }
      if (action === "play") await playback.playQueue(items);
      else if (action === "next") await playback.playNext(items, playlist.name);
      else await playback.addToQueue(items, playlist.name);
    } catch {
      toast("Could not play this playlist.");
    }
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-60">
        <ContextMenuItem onSelect={() => void run("play")}>
          <Play className="h-4 w-4" />
          Play
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => void run("next")}>
          <ListStart className="h-4 w-4" />
          Play next
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => void run("end")}>
          <ListEnd className="h-4 w-4" />
          Add to queue
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem asChild>
          <Link href={`/playlist?id=${playlistId}`}>
            <ListMusic className="h-4 w-4" />
            View playlist
          </Link>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
