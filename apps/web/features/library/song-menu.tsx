"use client";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import CreatePlaylistDialog from "@/features/library/create-playlist-dialog";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { addSongToPlaylist, getPlaylists } from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Disc3,
  ListEnd,
  ListPlus,
  Loader2,
  Pause,
  Play,
  Plus,
  RefreshCw,
  UserRound,
  X,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";
import {
  songMenuEntityLinkVisibility,
  songMenuQueueItem,
  type SongMenuContext,
} from "./song-menu-state";

type SongMenuProps = {
  children: React.ReactNode;
  song_name: string;
  song_id: string;
  artist_id: string;
  artist_name: string;
  album_id: string;
  album_name: string;
  album_cover?: string;
  context?: SongMenuContext;
  onRemoveFromPlaylist?: () => void;
  removeFromPlaylistLabel?: string;
};

export default function SongMenu({
  children,
  song_name,
  song_id,
  artist_id,
  artist_name,
  album_id,
  album_name,
  album_cover,
  context = "default",
  onRemoveFromPlaylist,
  removeFromPlaylistLabel = "Remove from playlist",
}: SongMenuProps) {
  const playback = useZonePlayback();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const entityLinks = songMenuEntityLinkVisibility(context);
  const playlists = useQuery({
    queryKey: ["playlists"],
    queryFn: getPlaylists,
    enabled: open,
  });
  const addToPlaylist = useMutation({
    mutationFn: ({ id }: { id: number; name: string }) =>
      addSongToPlaylist(id, song_id),
    onSuccess: async (_, playlist) => {
      toast.success(`Added to ${playlist.name}`);
      await queryClient.invalidateQueries({
        queryKey: ["playlist", playlist.id],
        refetchType: "none",
      });
      await queryClient.invalidateQueries({
        queryKey: ["playlists"],
        refetchType: "none",
      });
    },
    onError: () => toast("Could not add this song."),
  });

  const queueItem = () => {
    return songMenuQueueItem({
      songId: song_id,
      songName: song_name,
      artistId: artist_id,
      artistName: artist_name,
      albumId: album_id,
      albumName: album_name,
      albumCover: album_cover,
    });
  };

  const play = () => {
    if (context === "player") {
      void playback.togglePlayPause();
      return;
    }
    void playback.playQueue([queueItem()]);
  };

  const addToQueue = () => {
    void playback.addToQueue([queueItem()], song_name);
  };

  const playNext = () => {
    void playback.playNext([queueItem()], song_name);
  };

  return (
    <>
      <ContextMenu onOpenChange={setOpen}>
        <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
        <ContextMenuContent className="w-60">
          <ContextMenuItem onSelect={play}>
            {context === "player" && playback.isPlaying ? (
              <Pause className="h-4 w-4" />
            ) : (
              <Play className="h-4 w-4" />
            )}
            {context === "player" && playback.isPlaying ? "Pause" : "Play"}
          </ContextMenuItem>
          {(context === "default" || context === "album") && (
            <ContextMenuItem onSelect={playNext}>
              <ListPlus className="h-4 w-4" />
              Play next
            </ContextMenuItem>
          )}
          {context !== "artist" && (
            <ContextMenuItem onSelect={addToQueue}>
              <ListEnd className="h-4 w-4" />
              Add to queue
            </ContextMenuItem>
          )}
          {context !== "artist" && <ContextMenuSeparator />}
          {context !== "artist" && (
            <ContextMenuSub>
              <ContextMenuSubTrigger>
                <ListPlus className="h-4 w-4" />
                Add to playlist
              </ContextMenuSubTrigger>
              <ContextMenuSubContent className="w-52">
                {playlists.isPending && (
                  <ContextMenuItem disabled>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Loading playlists…
                  </ContextMenuItem>
                )}
                {playlists.isError && (
                  <ContextMenuItem onSelect={() => void playlists.refetch()}>
                    <RefreshCw className="h-4 w-4" />
                    Try loading again
                  </ContextMenuItem>
                )}
                {playlists.data?.map((playlist) => (
                  <ContextMenuItem
                    disabled={addToPlaylist.isPending}
                    key={playlist.id}
                    onSelect={() =>
                      addToPlaylist.mutate({
                        id: playlist.id,
                        name: playlist.name,
                      })
                    }
                  >
                    <ListPlus className="h-4 w-4" />
                    <span className="truncate">{playlist.name}</span>
                  </ContextMenuItem>
                ))}
                {playlists.isSuccess && Boolean(playlists.data?.length) && (
                  <ContextMenuSeparator />
                )}
                {playlists.isSuccess && (
                  <ContextMenuItem onSelect={() => setCreateOpen(true)}>
                    <Plus className="h-4 w-4" />
                    New playlist
                  </ContextMenuItem>
                )}
              </ContextMenuSubContent>
            </ContextMenuSub>
          )}
          {(entityLinks.album || entityLinks.artist) && (
            <ContextMenuSeparator />
          )}
          {entityLinks.album && (
            <ContextMenuItem asChild>
              <Link href={`/album?id=${album_id}`}>
                <Disc3 className="h-4 w-4" />
                View album
              </Link>
            </ContextMenuItem>
          )}
          {entityLinks.artist && (
            <ContextMenuItem asChild>
              <Link href={`/artist?id=${artist_id}`}>
                <UserRound className="h-4 w-4" />
                View artist
              </Link>
            </ContextMenuItem>
          )}
          {onRemoveFromPlaylist && <ContextMenuSeparator />}
          {onRemoveFromPlaylist && (
            <ContextMenuItem
              data-variant="destructive"
              onSelect={onRemoveFromPlaylist}
            >
              <X className="h-4 w-4" />
              {removeFromPlaylistLabel}
            </ContextMenuItem>
          )}
        </ContextMenuContent>
      </ContextMenu>
      <CreatePlaylistDialog
        initialSongIds={[song_id]}
        onOpenChange={setCreateOpen}
        open={createOpen}
      />
    </>
  );
}
