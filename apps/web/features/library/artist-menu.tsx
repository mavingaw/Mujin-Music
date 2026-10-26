"use client";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { usePlayer } from "@/features/player/player-context";
import { getArtistInfo } from "@parson/music-sdk";
import { Play, Radio, UserRound } from "lucide-react";
import { useStartRadio } from "@/features/player/radio/radio-controls";
import Link from "next/link";
import { BanMenuItem } from "@/features/library/radio-bans";
import { toast } from "sonner";
import { useState } from "react";
import {
  CreateTagDialog,
  TagSubmenu,
} from "@/features/library/focus/tag-submenu";

export default function ArtistMenu({
  artistId,
  artistName,
  children,
}: {
  artistId: string;
  artistName?: string;
  children: React.ReactNode;
}) {
  const player = usePlayer();
  const startRadio = useStartRadio();
  const [createTagOpen, setCreateTagOpen] = useState(false);

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
      const first = queue[0];
      if (!first) {
        return;
      }
      player.setQueue(queue);
      player.setCurrentSongIndex(0);
      player.setSongCallback(first.song, artist, first.album);
      player.playAudioSource();
    } catch {
      toast("Could not play this artist.");
    }
  };

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
        <ContextMenuContent className="w-60">
          <ContextMenuItem onSelect={() => void play()}>
            <Play className="h-4 w-4" />
            Play
          </ContextMenuItem>
          <ContextMenuItem
            onSelect={() => void startRadio({ kind: "artist", id: artistId })}
          >
            <Radio className="h-4 w-4" />
            Start radio
          </ContextMenuItem>
          <BanMenuItem
            target={{ kind: "artist", id: artistId, name: artistName }}
          />
          <ContextMenuSeparator />
          <ContextMenuItem asChild>
            <Link href={`/artist?id=${artistId}`}>
              <UserRound className="h-4 w-4" />
              View artist
            </Link>
          </ContextMenuItem>
          <TagSubmenu
            entityId={artistId}
            entityType="artist"
            onCreateTag={() => setCreateTagOpen(true)}
          />
        </ContextMenuContent>
      </ContextMenu>
      {createTagOpen && (
        <CreateTagDialog
          entityId={artistId}
          entityType="artist"
          onOpenChange={setCreateTagOpen}
          open={createTagOpen}
        />
      )}
    </>
  );
}
