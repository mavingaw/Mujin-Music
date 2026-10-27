import {
  getSongWorks,
  trackWorkTarget,
  type LibrarySong,
  type ResponseAlbum,
  type WorkSummary,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import {
  Disc3,
  Download,
  ChevronDown,
  ChevronUp,
  ListEnd,
  ListPlus,
  MoreHorizontal,
  Music2,
  Play,
  X,
  UserRound,
} from "lucide-react-native";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { Artwork } from "@/components/artwork";
import { ActionDrawer, DrawerAction } from "@/components/action-drawer";
import { StartRadioAction } from "@/components/radio-ui";
import { BanAction, BannedMark } from "@/components/radio-bans";
import { ListenLaterAction } from "@/components/listen-later";
import { TagsAction } from "@/components/tag-picker";
import { palette } from "@/constants/colors";
import { usePlayer } from "@/providers/player-provider";
import { useZonePlayback } from "@/hooks/use-zone-playback";
import {
  downloadSong,
  isSongDownloaded,
  removeDownload,
  useDownloadsRevision,
} from "@/lib/downloads";
import { AlbumActions } from "@/components/album-actions";
import { PlaylistPicker } from "@/components/playlist-picker";
import { useSession } from "@/providers/session-provider";
import { trackArtistCredit } from "@/lib/track-artist-credit";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

export function Screen({ children }: { children?: React.ReactNode }) {
  return <View style={styles.screen}>{children}</View>;
}

export function PageTitle({
  children,
  subtitle,
}: {
  children: React.ReactNode;
  subtitle?: string;
}) {
  return (
    <View style={styles.titleBlock}>
      <Text accessibilityRole="header" style={styles.pageTitle}>
        {children}
      </Text>
      {subtitle ? <Text style={styles.subtitle}>{subtitle}</Text> : null}
    </View>
  );
}

export function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <Text accessibilityRole="header" style={styles.sectionTitle}>
      {children}
    </Text>
  );
}

export function SongRow({
  song,
  queue,
  index,
  onRemove,
  onMoveDown,
  onMoveUp,
  removeLabel = "Remove from playlist",
  showAlbum = true,
  albumArtist,
  albumType,
  work,
  title,
}: {
  song: LibrarySong;
  queue?: LibrarySong[];
  index?: number;
  onRemove?: () => void;
  onMoveDown?: () => void;
  onMoveUp?: () => void;
  removeLabel?: string;
  showAlbum?: boolean;
  albumArtist?: string;
  albumType?: string | null;
  /** The piece the song performs when known; looked up when undefined. */
  work?: WorkSummary | null;
  /** Shown instead of the song name, e.g. a movement under a work heading. */
  title?: string;
}) {
  const player = usePlayer();
  const playback = useZonePlayback();
  const session = useSession();
  const router = useRouter();
  const [menu, setMenu] = useState(false);
  const [playlistPicker, setPlaylistPicker] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [downloadError, setDownloadError] = useState(false);
  useDownloadsRevision();
  const active = player.current?.id === song.id;
  const songWorks = useQuery({
    queryKey: ["song-works", song.id],
    queryFn: () => getSongWorks(song.id),
    enabled: menu && work === undefined && session.phase !== "offline",
    staleTime: 5 * 60_000,
    retry: false,
  });
  const piece = work === undefined ? trackWorkTarget(songWorks.data) : work;
  const albumCredit =
    !showAlbum && albumArtist
      ? trackArtistCredit({
          albumArtist,
          albumType,
          contributingArtists: song.contributing_artists,
          trackArtist: song.artist,
        })
      : null;
  const subtitle = showAlbum
    ? `${song.artist}${
        song.album_object?.name ? ` • ${song.album_object.name}` : ""
      }`
    : albumCredit;
  return (
    <View style={styles.songRow}>
      <Pressable
        {...immediatePressFeedback}
        accessibilityLabel={`${song.name} by ${song.artist}`}
        accessibilityRole="button"
        style={({ pressed }) => [styles.songMain, pressed && styles.pressed]}
        onPress={() => {
          if (menu) return;
          if (active) player.toggle();
          else playback.playSong(song, queue);
        }}
        onLongPress={() => setMenu(true)}
        delayLongPress={220}
      >
        {index !== undefined ? (
          <Text style={styles.trackNumber}>{index + 1}</Text>
        ) : (
          <Artwork path={song.album_object?.cover_url} size={48} />
        )}
        <View style={styles.songLabels}>
          <View style={styles.songTitleRow}>
            <Text numberOfLines={1} style={[styles.songTitle, styles.shrink]}>
              {title ?? song.name}
            </Text>
            <BannedMark target={{ kind: "song", id: song.id }} />
          </View>
          {subtitle ? (
            <Text numberOfLines={1} style={styles.songArtist}>
              {subtitle}
            </Text>
          ) : null}
        </View>
      </Pressable>
      <Pressable
        {...immediateBorderlessPressFeedback}
        accessibilityLabel={`More actions for ${song.name}`}
        accessibilityRole="button"
        hitSlop={12}
        onPress={() => setMenu(true)}
        style={styles.rowMenuButton}
      >
        <MoreHorizontal color={palette.secondary} size={20} />
      </Pressable>
      <ActionDrawer
        open={menu}
        onClose={() => setMenu(false)}
        title={song.name}
      >
        <DrawerAction
          icon={Play}
          label="Play"
          onPress={() => {
            setMenu(false);
            playback.playSong(song, queue);
          }}
        />
        <DrawerAction
          icon={ListPlus}
          label="Play next"
          onPress={() => {
            setMenu(false);
            playback.addNext(song);
          }}
        />
        <DrawerAction
          icon={ListEnd}
          label="Add to queue"
          onPress={() => {
            setMenu(false);
            playback.addToQueue([song]);
          }}
        />
        {session.phase !== "offline" ? (
          <StartRadioAction
            seed={{ kind: "song", id: song.id }}
            onStarted={() => setMenu(false)}
          />
        ) : null}
        <BanAction
          target={{ kind: "song", id: song.id, name: song.name }}
          onDone={() => setMenu(false)}
        />
        <ListenLaterAction
          entityId={song.id}
          entityType="track"
          onDone={() => setMenu(false)}
        />
        <TagsAction entityId={song.id} entityType="track" title={song.name} />
        {session.phase !== "offline" && piece ? (
          <DrawerAction
            icon={Music2}
            label="Go to work"
            onPress={() => {
              setMenu(false);
              router.push(`/work/${encodeURIComponent(piece.mbid)}`);
            }}
          />
        ) : null}
        {session.phase !== "offline" && song.album_object?.id ? (
          <DrawerAction
            icon={Disc3}
            label="View album"
            onPress={() => {
              setMenu(false);
              router.push(`/album/${song.album_object.id}`);
            }}
          />
        ) : null}
        {session.phase !== "offline" && song.artist_object?.id ? (
          <DrawerAction
            icon={UserRound}
            label="View artist"
            onPress={() => {
              setMenu(false);
              router.push(`/artist/${song.artist_object.id}`);
            }}
          />
        ) : null}
        {session.phase !== "offline" ? (
          <DrawerAction
            icon={ListPlus}
            label="Add to playlist"
            onPress={() => {
              setMenu(false);
              setPlaylistPicker(true);
            }}
          />
        ) : null}
        {session.phase !== "offline" || isSongDownloaded(song.id) ? (
          <DrawerAction
            icon={isSongDownloaded(song.id) ? X : Download}
            label={
              isSongDownloaded(song.id)
                ? "Delete from device"
                : downloading
                  ? "Downloading song…"
                  : downloadError
                    ? "Download failed · Try again"
                    : "Download song"
            }
            onPress={() => {
              if (downloading) return;
              if (isSongDownloaded(song.id)) {
                void removeDownload(song.id)
                  .then(() => setMenu(false))
                  .catch(() => setDownloadError(true));
                return;
              }
              setDownloadError(false);
              setDownloading(true);
              void downloadSong(song)
                .then(() => setMenu(false))
                .catch(() => setDownloadError(true))
                .finally(() => setDownloading(false));
            }}
          />
        ) : null}
        {downloadError ? (
          <Text accessibilityRole="alert" style={styles.actionError}>
            The download action failed. Please try again.
          </Text>
        ) : null}
        {onMoveUp ? (
          <DrawerAction
            icon={ChevronUp}
            label="Move up"
            onPress={() => {
              setMenu(false);
              onMoveUp();
            }}
          />
        ) : null}
        {onMoveDown ? (
          <DrawerAction
            icon={ChevronDown}
            label="Move down"
            onPress={() => {
              setMenu(false);
              onMoveDown();
            }}
          />
        ) : null}
        {onRemove ? (
          <DrawerAction
            icon={X}
            label={removeLabel}
            onPress={() => {
              setMenu(false);
              onRemove();
            }}
          />
        ) : null}
      </ActionDrawer>
      <PlaylistPicker
        open={session.phase !== "offline" && playlistPicker}
        onClose={() => setPlaylistPicker(false)}
        songId={song.id}
      />
    </View>
  );
}

export function AlbumRail({ albums }: { albums: ResponseAlbum[] }) {
  const router = useRouter();
  const [selected, setSelected] = useState<ResponseAlbum | null>(null);
  return (
    <>
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={styles.rail}
      >
        {albums.map((album) => (
          <Pressable
            accessibilityLabel={`${album.name} by ${
              album.artist_object?.name ??
              album.contributing_artists?.[0] ??
              "Unknown artist"
            }`}
            accessibilityRole="button"
            key={album.id}
            style={styles.card}
            onPress={() => {
              if (!selected) router.push(`/album/${album.id}`);
            }}
            onLongPress={() => setSelected(album)}
          >
            <Artwork path={album.cover_url} size={154} rounded={10} />
            <Text numberOfLines={1} style={styles.cardTitle}>
              {album.name}
            </Text>
            <Text numberOfLines={1} style={styles.cardSubtitle}>
              {album.artist_object?.name ??
                album.contributing_artists?.[0] ??
                "Album"}
            </Text>
          </Pressable>
        ))}
      </ScrollView>
      {selected ? (
        <AlbumActions
          open
          albumId={selected.id}
          artistId={selected.artist_object?.id}
          name={selected.name}
          loaded={selected}
          onClose={() => setSelected(null)}
        />
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  songTitleRow: { alignItems: "center", flexDirection: "row", gap: 6 },
  shrink: { flexShrink: 1 },
  screen: { flex: 1, backgroundColor: palette.background },
  titleBlock: { paddingHorizontal: 20, paddingTop: 12, paddingBottom: 14 },
  pageTitle: {
    color: "white",
    fontSize: 30,
    lineHeight: 36,
    fontWeight: "900",
    letterSpacing: -1,
  },
  subtitle: { color: palette.secondary, marginTop: 4, fontSize: 14 },
  sectionTitle: {
    color: "white",
    fontSize: 21,
    fontWeight: "800",
    letterSpacing: -0.4,
    marginHorizontal: 20,
    marginTop: 18,
    marginBottom: 13,
  },
  songRow: {
    minHeight: 64,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 20,
    paddingRight: 8,
  },
  songMain: {
    minHeight: 64,
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  pressed: { opacity: 0.62 },
  actionError: { color: "#ff9b9b", paddingHorizontal: 16, paddingVertical: 8 },
  trackNumber: {
    color: palette.secondary,
    width: 24,
    textAlign: "center",
    fontVariant: ["tabular-nums"],
  },
  songLabels: { flex: 1, justifyContent: "center" },
  songTitle: { color: "white", fontSize: 15, fontWeight: "600" },
  songArtist: { color: palette.secondary, fontSize: 13, marginTop: 3 },
  rowMenuButton: {
    width: 36,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  rail: { paddingHorizontal: 20, gap: 15 },
  card: { width: 154 },
  cardTitle: { color: "white", fontWeight: "700", fontSize: 14, marginTop: 9 },
  cardSubtitle: { color: palette.secondary, fontSize: 13, marginTop: 3 },
});
