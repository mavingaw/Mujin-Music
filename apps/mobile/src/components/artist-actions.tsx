import { getArtistInfo, type LibrarySong } from "@parson/music-sdk";
import { ListEnd, ListPlus, Play, UserRound } from "lucide-react-native";
import { useRouter } from "expo-router";
import { useState } from "react";

import { ActionDrawer, DrawerAction } from "@/components/action-drawer";
import { StartRadioAction } from "@/components/radio-ui";
import { BanAction } from "@/components/radio-bans";
import { ListenLaterAction } from "@/components/listen-later";
import { TagsAction } from "@/components/tag-picker";
import { useZonePlayback } from "@/hooks/use-zone-playback";

export function ArtistActions({
  artistId,
  name,
  onClose,
  open,
}: {
  artistId: string;
  name?: string;
  onClose: () => void;
  open: boolean;
}) {
  const router = useRouter();
  const playback = useZonePlayback();
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  // Loads the artist's tracks (each once), then plays or queues them.
  const run = (action: (songs: LibrarySong[]) => void) => {
    if (loading) return;
    setFailed(false);
    setLoading(true);
    void getArtistInfo(artistId)
      .then((artist) => {
        const seen = new Set<string>();
        const songs = artist.albums
          .flatMap((album) => album.songs as LibrarySong[])
          .filter((song) => !seen.has(song.id) && Boolean(seen.add(song.id)));
        if (songs.length) action(songs);
        onClose();
      })
      .catch(() => setFailed(true))
      .finally(() => setLoading(false));
  };
  return (
    <ActionDrawer open={open} onClose={onClose} title={name}>
      <DrawerAction
        icon={Play}
        label={
          loading
            ? "Loading artist…"
            : failed
              ? "Try playing again"
              : "Play now"
        }
        onPress={() =>
          run((songs) => {
            if (songs[0]) playback.playSong(songs[0], songs);
          })
        }
      />
      <DrawerAction
        icon={ListPlus}
        label="Add next"
        onPress={() => run((songs) => playback.addNext(songs))}
      />
      <DrawerAction
        icon={ListEnd}
        label="Add to queue"
        onPress={() => run((songs) => playback.addToQueue(songs))}
      />
      <StartRadioAction
        seed={{ kind: "artist", id: artistId }}
        onStarted={onClose}
      />
      <BanAction
        target={{ kind: "artist", id: artistId, name }}
        onDone={onClose}
      />
      <ListenLaterAction
        entityId={artistId}
        entityType="artist"
        onDone={onClose}
      />
      <TagsAction entityId={artistId} entityType="artist" title={name} />
      <DrawerAction
        icon={UserRound}
        label="Go to artist"
        onPress={() => {
          onClose();
          router.push(`/artist/${artistId}`);
        }}
      />
    </ActionDrawer>
  );
}
