"use client";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { getArtistInfo } from "@parson/music-sdk";
import { Play, UserRound } from "lucide-react";
import Link from "next/link";
import { toast } from "sonner";

export default function ArtistMenu({
  artistId,
  children,
}: {
  artistId: string;
  children: React.ReactNode;
}) {
  const playback = useZonePlayback();

  const play = async () => {
    try {
      const artist = await getArtistInfo(artistId);
      const seen = new Set<string>();
      const queue = artist.albums.flatMap((album) =>
        album.songs
          .filter((song) => {
            if (seen.has(song.id)) {
              return false;
            }
            seen.add(song.id);
            return true;
          })
          .map((song) => ({ song, album, artist })),
      );
      if (!queue.length) {
        return;
      }
      await playback.playQueue(queue);
    } catch {
      toast("Could not play this artist.");
    }
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-60">
        <ContextMenuItem onSelect={() => void play()}>
          <Play className="h-4 w-4" />
          Play
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem asChild>
          <Link href={`/artist?id=${artistId}`}>
            <UserRound className="h-4 w-4" />
            View artist
          </Link>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
