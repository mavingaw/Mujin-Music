import {
  STREAMING_PROVIDER_NAMES,
  getStreamingAlbum,
  streamingTrackToSong,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { ArrowLeft, Play } from "lucide-react-native";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import { Screen } from "@/components/music-ui";
import { PauseGlyph } from "@/components/pause-glyph";
import {
  FormatTag,
  LibraryToggle,
  SourceTag,
  StreamingTrackRow,
} from "@/components/streaming-ui";
import {
  AlbumCreditsSection,
  AwardTags,
  EditorialSection,
  StartRadioButton,
} from "@/components/streaming-notes";
import {
  PreferredPlayCaption,
  usePreferredAlbumPlay,
} from "@/components/preferred-album-play";
import { palette } from "@/constants/colors";
import { formatCollectionDuration } from "@/lib/format";

/** A TIDAL or Qobuz album, opened from search, the library or the player. */
export function StreamingAlbumScreen({ id }: { id: string }) {
  const router = useRouter();
  const album = useQuery({
    queryKey: ["streaming", "album", id],
    queryFn: () => getStreamingAlbum(id),
    retry: 1,
  });
  const loaded = album.data;
  const songs =
    loaded?.tracks
      .filter((track) => track.streamable)
      .map((track) => streamingTrackToSong(track, loaded)) ?? [];
  const preferred = usePreferredAlbumPlay(
    loaded
      ? {
          id: loaded.id,
          title: loaded.title,
          artistName: loaded.artist.name,
          source: loaded.provider,
          format: loaded.format ?? null,
        }
      : null,
    songs,
  );
  if (album.isPending)
    return (
      <Screen>
        <ActivityIndicator color={palette.text} style={{ flex: 1 }} />
      </Screen>
    );
  const data = album.data;
  if (!data) {
    const message = (
      album.error as { response?: { data?: { message?: string } } } | null
    )?.response?.data?.message;
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          onPress={() => void album.refetch()}
          style={styles.errorState}
        >
          <Text style={styles.error}>
            {message ?? "Could not load this album"} · Tap to retry
          </Text>
        </Pressable>
      </Screen>
    );
  }
  const playing = preferred.playing;
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <View style={styles.nav}>
          <Pressable
            accessibilityLabel="Back"
            accessibilityRole="button"
            hitSlop={12}
            onPress={router.back}
          >
            <ArrowLeft color={palette.text} size={25} />
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={{ paddingBottom: 135 }}>
          <View style={styles.hero}>
            <Artwork
              path={data.cover_url}
              rounded={12}
              size={260}
              style={styles.cover}
            />
            <View style={styles.tags}>
              <SourceTag provider={data.provider} />
              {data.format ? <FormatTag label={data.format.label} /> : null}
            </View>
            <Text style={styles.title}>{data.title}</Text>
            <Pressable
              accessibilityLabel={`View ${data.artist.name}`}
              accessibilityRole="button"
              onPress={() =>
                router.push(`/artist/${encodeURIComponent(data.artist.id)}`)
              }
            >
              <Text style={styles.artist}>{data.artist.name}</Text>
            </Pressable>
            <Text style={styles.meta}>
              {[
                data.release_date?.slice(0, 4),
                `${data.tracks.length} tracks, ${formatCollectionDuration(data.duration)}`,
                data.genre,
              ]
                .filter(Boolean)
                .join(" • ")}
            </Text>
            <AwardTags awards={data.awards} />
            <View style={styles.actions}>
              <Pressable
                accessibilityLabel={playing ? "Pause album" : "Play album"}
                accessibilityRole="button"
                disabled={
                  (!songs.length && !preferred.choice) || preferred.starting
                }
                onPress={() => void preferred.play()}
                style={styles.circle}
              >
                {playing ? (
                  <PauseGlyph color={palette.background} size={25} />
                ) : (
                  <Play
                    color={palette.background}
                    fill={palette.background}
                    size={28}
                  />
                )}
              </Pressable>
              <LibraryToggle
                id={data.id}
                inLibrary={data.in_library}
                kind="album"
                title={data.title}
                wide
              />
              <StartRadioButton id={data.id} label={data.title} seed="album" />
            </View>
            <PreferredPlayCaption play={preferred} />
          </View>
          {data.tracks.map((track, index) => (
            <StreamingTrackRow
              album={data}
              index={index}
              key={track.id}
              queue={data.tracks}
              track={track}
            />
          ))}
          {data.review ? (
            <EditorialSection text={data.review} title="About this album" />
          ) : null}
          <AlbumCreditsSection album={data} />
          <Text style={styles.footer}>
            {[data.label, data.copyright].filter(Boolean).join("\n")}
            {"\n"}Streams from {STREAMING_PROVIDER_NAMES[data.provider]} at the
            quality chosen in Settings, Services.
          </Text>
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: { height: 50, paddingHorizontal: 20, justifyContent: "center" },
  hero: { paddingHorizontal: 20, alignItems: "flex-start" },
  cover: { alignSelf: "center", marginVertical: 12 },
  tags: { flexDirection: "row", alignItems: "center", gap: 8, marginTop: 14 },
  title: {
    color: palette.text,
    fontSize: 30,
    lineHeight: 34,
    fontWeight: "900",
    letterSpacing: -0.8,
    marginTop: 10,
  },
  artist: {
    color: palette.text,
    fontWeight: "800",
    fontSize: 15,
    marginTop: 9,
  },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 5 },
  actions: {
    width: "100%",
    flexDirection: "row",
    alignItems: "center",
    gap: 16,
    marginVertical: 22,
  },
  circle: {
    width: 56,
    height: 56,
    borderRadius: 28,
    backgroundColor: palette.text,
    alignItems: "center",
    justifyContent: "center",
  },
  footer: { color: palette.muted, fontSize: 12, lineHeight: 18, margin: 20 },
  error: { color: palette.secondary, margin: 30, textAlign: "center" },
  errorState: { flex: 1, alignItems: "center", justifyContent: "center" },
});
