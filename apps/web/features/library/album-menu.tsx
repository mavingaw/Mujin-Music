"use client";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useSession } from "@/features/account/session-provider";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { getAlbumInfo, type LibraryAlbum } from "@parson/music-sdk";
import {
  Disc3,
  Download,
  ListEnd,
  ListStart,
  Play,
  UserRound,
} from "lucide-react";
import { downloadAlbum } from "@/lib/downloads/download-album";
import { toast } from "sonner";
import Link from "next/link";

type AlbumMenuProps = {
  album_id: string;
  artist_id: string;
  children: React.ReactNode;
  showArtist?: boolean;
};

export default function AlbumMenu({
  album_id,
  artist_id,
  children,
  showArtist = true,
}: AlbumMenuProps) {
  const playback = useZonePlayback();
  const { session } = useSession();

  const loadAlbum = () =>
    getAlbumInfo(album_id, false) as Promise<LibraryAlbum>;

  const run = async (action: "play" | "next" | "end") => {
    let album: LibraryAlbum;
    try {
      album = await loadAlbum();
    } catch {
      toast("Could not load this album.");
      return;
    }
    const artist = album.artist_object;
    if (!album.songs?.length || !artist) {
      return;
    }
    const items = album.songs.map((song) => ({ song, album, artist }));
    if (action === "play") await playback.playQueue(items);
    else if (action === "next") await playback.playNext(items, album.name);
    else await playback.addToQueue(items, album.name);
  };

  const save = async () => {
    const album = await loadAlbum();
    const count = downloadAlbum(album);
    if (count) {
      toast.success(`Downloading ${count} songs from ${album.name}`);
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
          <Link href={`/album?id=${album_id}`}>
            <Disc3 className="h-4 w-4" />
            View album
          </Link>
        </ContextMenuItem>
        {session?.role === "user" && (
          <ContextMenuItem onSelect={() => void save()}>
            <Download className="h-4 w-4" />
            Download album
          </ContextMenuItem>
        )}
        {showArtist && (
          <ContextMenuItem asChild>
            <Link href={`/artist?id=${artist_id}`}>
              <UserRound className="h-4 w-4" />
              View artist
            </Link>
          </ContextMenuItem>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}
