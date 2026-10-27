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
import { BanMenuItem } from "@/features/library/radio-bans";
import {
  CreateTagDialog,
  TagSubmenu,
} from "@/features/library/focus/tag-submenu";
import { ListenLaterMenuItem } from "@/features/library/listen-later/listen-later-menu-item";
import { useStartRadio } from "@/features/player/radio/radio-controls";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import {
  addSongToPlaylist,
  getPlaylists,
  trackWorkTarget,
  type WorkSummary,
} from "@parson/music-sdk";
import { useSongWorks } from "@/features/library/works/use-works";
import { LibraryToolsSubmenu } from "@/features/library/tools/library-tools-menu";
import { SongAnalysisDialog } from "@/features/library/tools/song-analysis-dialog";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Disc3,
  Info,
  Music2,
  ListEnd,
  ListPlus,
  Loader2,
  Pause,
  Play,
  Plus,
  Radio,
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
  /** Shows "View credits…" when the caller can show the track's credits. */
  onViewCredits?: () => void;
  /** The piece the track performs, when the caller already knows it.
   * Left undefined, the menu looks it up when it opens. */
  work?: WorkSummary | null;
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
  onViewCredits,
  work,
}: SongMenuProps) {
  const playback = useZonePlayback();
  const startRadio = useStartRadio();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [createTagOpen, setCreateTagOpen] = useState(false);
  const [analysisOpen, setAnalysisOpen] = useState(false);
  const entityLinks = songMenuEntityLinkVisibility(context);
  const songWorks = useSongWorks(song_id, open && work === undefined);
  const piece = work === undefined ? trackWorkTarget(songWorks.data) : work;
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
          <ContextMenuItem
            onSelect={() => void startRadio({ kind: "song", id: song_id })}
          >
            <Radio className="h-4 w-4" />
            Start radio
          </ContextMenuItem>
          <BanMenuItem
            target={{ kind: "song", id: song_id, name: song_name }}
          />
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
          <ListenLaterMenuItem
            entityId={song_id}
            entityType="track"
            name={song_name}
          />
          {context !== "player" && (
            <TagSubmenu
              entityId={song_id}
              entityType="track"
              onCreateTag={() => setCreateTagOpen(true)}
            />
          )}
          {(entityLinks.album ||
            entityLinks.artist ||
            onViewCredits ||
            piece) && <ContextMenuSeparator />}
          {onViewCredits && (
            <ContextMenuItem onSelect={onViewCredits}>
              <Info className="h-4 w-4" />
              View credits…
            </ContextMenuItem>
          )}
          {piece && (
            <ContextMenuItem asChild>
              <Link href={`/work?id=${encodeURIComponent(piece.mbid)}`}>
                <Music2 className="h-4 w-4" />
                <span className="truncate">Go to work</span>
              </Link>
            </ContextMenuItem>
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
          {context !== "player" && (
            <LibraryToolsSubmenu
              onShowAnalysis={() => setAnalysisOpen(true)}
              scope={{ scope: "songs", ids: [song_id] }}
            />
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
      {analysisOpen && (
        <SongAnalysisDialog
          onOpenChange={setAnalysisOpen}
          open={analysisOpen}
          songId={song_id}
        />
      )}
      <CreatePlaylistDialog
        initialSongIds={[song_id]}
        onOpenChange={setCreateOpen}
        open={createOpen}
      />
      {createTagOpen && (
        <CreateTagDialog
          entityId={song_id}
          entityType="track"
          onOpenChange={setCreateTagOpen}
          open={createTagOpen}
        />
      )}
    </>
  );
}
