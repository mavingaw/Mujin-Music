import {
  STREAMING_PROVIDER_NAMES,
  addToStreamingLibrary,
  removeFromStreamingLibrary,
  streamingTrackToSong,
  type StreamingAlbum,
  type StreamingItemKind,
  type StreamingProviderId,
  type StreamingTrack,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Check, Plus } from "lucide-react-native";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { Artwork } from "@/components/artwork";
import { palette } from "@/constants/colors";
import { formatDuration } from "@/lib/format";
import { usePlayer } from "@/providers/player-provider";

/** Neutral monogram plus the service's name; Parson does not use their logos. */
export function SourceTag({
  provider,
  large = false,
}: {
  provider: StreamingProviderId;
  large?: boolean;
}) {
  return (
    <View style={styles.sourceTag}>
      <View
        style={[
          styles.mark,
          large && styles.markLarge,
          provider === "qobuz" ? styles.qobuzMark : styles.tidalMark,
        ]}
      >
        <Text
          style={[
            styles.markText,
            large && styles.markTextLarge,
            provider === "tidal" && { color: "black" },
          ]}
        >
          {provider === "tidal" ? "T" : "Q"}
        </Text>
      </View>
      <Text style={[styles.sourceName, large && styles.sourceNameLarge]}>
        {STREAMING_PROVIDER_NAMES[provider]}
      </Text>
    </View>
  );
}

export function FormatTag({ label }: { label: string }) {
  const hiRes = /^24\b|^24-bit/i.test(label);
  return (
    <Text style={[styles.format, hiRes && styles.formatHiRes]}>{label}</Text>
  );
}

export function LibraryToggle({
  kind,
  id,
  inLibrary,
  title,
  wide = false,
}: {
  kind: StreamingItemKind;
  id: string;
  inLibrary: boolean;
  title: string;
  wide?: boolean;
}) {
  const queryClient = useQueryClient();
  const toggle = useMutation({
    mutationFn: () =>
      inLibrary
        ? removeFromStreamingLibrary(kind, id)
        : addToStreamingLibrary(kind, id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["streaming"] }),
  });
  const showIn = toggle.isPending ? !inLibrary : inLibrary;
  const Icon = showIn ? Check : Plus;
  return (
    <Pressable
      accessibilityLabel={
        showIn ? `Remove ${title} from library` : `Add ${title} to library`
      }
      accessibilityRole="button"
      accessibilityState={{ selected: showIn, busy: toggle.isPending }}
      disabled={toggle.isPending}
      hitSlop={10}
      onPress={() => toggle.mutate()}
      style={wide ? styles.libraryWide : styles.libraryCompact}
    >
      {toggle.isPending ? (
        <ActivityIndicator color="white" size="small" />
      ) : (
        <Icon
          color={showIn ? palette.success : "white"}
          size={wide ? 17 : 18}
        />
      )}
      {wide ? (
        <Text style={styles.libraryWideText}>
          {showIn ? "In library" : "Add to library"}
        </Text>
      ) : null}
    </Pressable>
  );
}

export function StreamingTrackRow({
  track,
  queue,
  album,
  index,
}: {
  track: StreamingTrack;
  queue: StreamingTrack[];
  album?: StreamingAlbum;
  index?: number;
}) {
  const player = usePlayer();
  const song = streamingTrackToSong(track, album);
  const active = player.current?.id === track.id;
  return (
    <View style={styles.row}>
      <Pressable
        accessibilityLabel={`${track.title} by ${track.artist.name}, ${STREAMING_PROVIDER_NAMES[track.provider]}`}
        accessibilityRole="button"
        disabled={!track.streamable}
        onPress={() => {
          if (active) player.toggle();
          else
            player.playSong(
              song,
              queue
                .filter((item) => item.streamable)
                .map((item) => streamingTrackToSong(item, album)),
            );
        }}
        style={({ pressed }) => [
          styles.rowMain,
          pressed && { opacity: 0.6 },
          !track.streamable && { opacity: 0.4 },
        ]}
      >
        {index !== undefined ? (
          <Text style={styles.trackNumber}>{index + 1}</Text>
        ) : (
          <Artwork path={track.album?.cover_url} size={48} />
        )}
        <View style={{ flex: 1 }}>
          <Text
            numberOfLines={1}
            style={[styles.title, active && { color: palette.success }]}
          >
            {track.title}
          </Text>
          <View style={styles.subtitleRow}>
            {index === undefined ? (
              <SourceTag provider={track.provider} />
            ) : null}
            <Text numberOfLines={1} style={styles.subtitle}>
              {track.artist.name}
              {index === undefined && track.album
                ? ` • ${track.album.title}`
                : ""}
            </Text>
          </View>
        </View>
        <Text style={styles.duration}>{formatDuration(track.duration)}</Text>
      </Pressable>
      <LibraryToggle
        id={track.id}
        inLibrary={track.in_library}
        kind="track"
        title={track.title}
      />
    </View>
  );
}

export function StreamingAlbumRow({ album }: { album: StreamingAlbum }) {
  const router = useRouter();
  return (
    <View style={styles.row}>
      <Pressable
        accessibilityLabel={`${album.title} by ${album.artist.name}, ${STREAMING_PROVIDER_NAMES[album.provider]} album`}
        accessibilityRole="button"
        onPress={() => router.push(`/album/${encodeURIComponent(album.id)}`)}
        style={({ pressed }) => [styles.rowMain, pressed && { opacity: 0.6 }]}
      >
        <Artwork path={album.cover_url} rounded={7} size={52} />
        <View style={{ flex: 1 }}>
          <Text numberOfLines={1} style={styles.title}>
            {album.title}
          </Text>
          <View style={styles.subtitleRow}>
            <SourceTag provider={album.provider} />
            <Text numberOfLines={1} style={styles.subtitle}>
              Album • {album.artist.name}
            </Text>
          </View>
          {album.format ? (
            <View style={{ marginTop: 5, flexDirection: "row" }}>
              <FormatTag label={album.format.label} />
            </View>
          ) : null}
        </View>
      </Pressable>
      <LibraryToggle
        id={album.id}
        inLibrary={album.in_library}
        kind="album"
        title={album.title}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  sourceTag: { flexDirection: "row", alignItems: "center", gap: 5 },
  mark: {
    width: 15,
    height: 15,
    borderRadius: 4,
    alignItems: "center",
    justifyContent: "center",
  },
  markLarge: { width: 22, height: 22, borderRadius: 6 },
  tidalMark: { backgroundColor: "white" },
  qobuzMark: { backgroundColor: "#3e8ef7" },
  markText: { color: "white", fontSize: 9, fontWeight: "900" },
  markTextLarge: { fontSize: 13 },
  sourceName: { color: palette.secondary, fontSize: 12, fontWeight: "700" },
  sourceNameLarge: { color: "white", fontSize: 15 },
  format: {
    color: "#bae6fd",
    fontSize: 11,
    fontWeight: "800",
    borderWidth: 1,
    borderColor: "rgba(125,211,252,0.3)",
    borderRadius: 5,
    paddingHorizontal: 5,
    paddingVertical: 1,
    overflow: "hidden",
  },
  formatHiRes: { color: "#fde68a", borderColor: "rgba(252,211,77,0.35)" },
  libraryCompact: {
    width: 40,
    height: 40,
    alignItems: "center",
    justifyContent: "center",
  },
  libraryWide: {
    height: 42,
    paddingHorizontal: 16,
    borderRadius: 21,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  libraryWideText: { color: "white", fontWeight: "700", fontSize: 14 },
  row: {
    minHeight: 64,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 20,
    paddingRight: 10,
  },
  rowMain: {
    minHeight: 64,
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 13,
  },
  trackNumber: {
    width: 24,
    color: palette.secondary,
    textAlign: "center",
    fontVariant: ["tabular-nums"],
  },
  title: { color: "white", fontWeight: "700", fontSize: 15 },
  subtitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    marginTop: 4,
  },
  subtitle: { color: palette.secondary, fontSize: 13, flexShrink: 1 },
  duration: {
    color: palette.secondary,
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
});
