import {
  createStreamingPlaylistCopy,
  creditLines,
  describeRadio,
  getStreamingRadio,
  parseStreamingId,
  streamingTrackToSong,
  type StreamingAlbum,
  type StreamingArtistSummary,
  type StreamingEditorialText,
  type StreamingRadioSeed,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Award, Radio } from "lucide-react-native";
import { useState } from "react";
import {
  ActivityIndicator,
  Alert,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { Artwork } from "@/components/artwork";
import { SectionTitle } from "@/components/music-ui";
import { palette } from "@/constants/colors";
import { editorialPreview } from "@/lib/streaming";
import { usePlayer } from "@/providers/player-provider";

/** Starts a TIDAL or Qobuz radio and offers to keep it as a Parson playlist. */
export function StartRadioButton({
  seed,
  id,
  label,
}: {
  seed: StreamingRadioSeed;
  id: string;
  label: string;
}) {
  const player = usePlayer();
  const queryClient = useQueryClient();
  const radio = useMutation({
    mutationFn: () => getStreamingRadio(seed, id),
    onSuccess: (result) => {
      const parsed = parseStreamingId(id);
      const tracks = result.tracks.filter((track) => track.streamable);
      const songs = tracks.map((track) => streamingTrackToSong(track));
      if (!parsed || !songs[0]) {
        Alert.alert("Nothing to play", "This radio has no playable tracks.");
        return;
      }
      player.playSong(songs[0], songs);
      const name = describeRadio(parsed.provider, result);
      Alert.alert(name, `${songs.length} tracks from ${label} are playing.`, [
        { text: "OK", style: "cancel" },
        {
          text: "Keep as playlist",
          onPress: () => {
            void createStreamingPlaylistCopy(parsed.provider, {
              title: `${label} radio`.slice(0, 200),
              description: `${name}, started from ${label}.`,
              track_ids: tracks.map((track) => track.id),
            })
              .then(() =>
                queryClient.invalidateQueries({
                  queryKey: ["streaming", "playlist-copies"],
                }),
              )
              .catch(() =>
                Alert.alert("Could not save", "Try keeping the radio again."),
              );
          },
        },
      ]);
    },
    onError: () =>
      Alert.alert("Could not start a radio", "Check your connection."),
  });
  return (
    <Pressable
      accessibilityLabel={`Start radio from ${label}`}
      accessibilityRole="button"
      accessibilityState={{ busy: radio.isPending }}
      disabled={radio.isPending}
      hitSlop={10}
      onPress={() => radio.mutate()}
      style={styles.radio}
    >
      {radio.isPending ? (
        <ActivityIndicator color="white" size="small" />
      ) : (
        <Radio color="white" size={17} />
      )}
      <Text style={styles.radioText}>Radio</Text>
    </Pressable>
  );
}

export function EditorialSection({
  title,
  text,
}: {
  title: string;
  text: StreamingEditorialText;
}) {
  const [expanded, setExpanded] = useState(false);
  const preview = editorialPreview(text.text, expanded);
  return (
    <View style={styles.section}>
      <SectionTitle>{title}</SectionTitle>
      <View style={styles.body}>
        {preview.paragraphs.map((paragraph, index) => (
          <Text key={index} style={styles.paragraph}>
            {paragraph}
          </Text>
        ))}
        <View style={styles.footerRow}>
          {preview.more ? (
            <Pressable
              accessibilityRole="button"
              hitSlop={8}
              onPress={() => setExpanded((value) => !value)}
            >
              <Text style={styles.more}>
                {expanded ? "Show less" : "Read more"}
              </Text>
            </Pressable>
          ) : null}
          {text.source ? (
            <Text style={styles.source}>Source: {text.source}</Text>
          ) : null}
        </View>
      </View>
    </View>
  );
}

export function AwardTags({ awards }: { awards: string[] | undefined }) {
  if (!awards?.length) return null;
  return (
    <View style={styles.awards}>
      {awards.map((award) => (
        <View key={award} style={styles.award}>
          <Award color="#f0dcc2" size={13} />
          <Text style={styles.awardText}>{award}</Text>
        </View>
      ))}
    </View>
  );
}

export function AlbumCreditsSection({ album }: { album: StreamingAlbum }) {
  const credited = album.tracks.filter((track) => track.credits?.length);
  if (!credited.length) return null;
  return (
    <View style={styles.section}>
      <SectionTitle>Credits</SectionTitle>
      <View style={styles.body}>
        {credited.map((track) => (
          <View key={track.id} style={styles.credit}>
            <Text numberOfLines={1} style={styles.creditTrack}>
              {track.track_number}. {track.title}
            </Text>
            {creditLines(track.credits).map((line) => (
              <Text key={line} style={styles.creditLine}>
                {line}
              </Text>
            ))}
          </View>
        ))}
      </View>
    </View>
  );
}

export function SimilarArtistsRow({
  artists,
}: {
  artists: StreamingArtistSummary[] | undefined;
}) {
  const router = useRouter();
  if (!artists?.length) return null;
  return (
    <View style={styles.section}>
      <SectionTitle>Similar artists</SectionTitle>
      <ScrollView
        contentContainerStyle={styles.similar}
        horizontal
        showsHorizontalScrollIndicator={false}
      >
        {artists.map((artist) => (
          <Pressable
            accessibilityLabel={`View ${artist.name}`}
            accessibilityRole="button"
            key={artist.id}
            onPress={() =>
              router.push(`/artist/${encodeURIComponent(artist.id)}`)
            }
            style={styles.similarItem}
          >
            <Artwork path={artist.picture_url} rounded={44} size={88} />
            <Text numberOfLines={1} style={styles.similarName}>
              {artist.name}
            </Text>
          </Pressable>
        ))}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  radio: {
    height: 42,
    paddingHorizontal: 16,
    borderRadius: 21,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  radioText: { color: "white", fontWeight: "700", fontSize: 14 },
  section: { marginTop: 18 },
  body: { paddingHorizontal: 20, gap: 10 },
  paragraph: { color: palette.secondary, fontSize: 14, lineHeight: 21 },
  footerRow: { flexDirection: "row", alignItems: "center", gap: 14 },
  more: { color: "white", fontWeight: "700", fontSize: 13 },
  source: { color: palette.muted, fontSize: 12 },
  awards: { flexDirection: "row", flexWrap: "wrap", gap: 8, marginTop: 10 },
  award: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: "rgba(240, 220, 194, 0.3)",
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  awardText: { color: "#f0dcc2", fontSize: 12, fontWeight: "700" },
  credit: { gap: 2 },
  creditTrack: { color: "white", fontWeight: "700", fontSize: 14 },
  creditLine: { color: palette.secondary, fontSize: 13, lineHeight: 19 },
  similar: { paddingHorizontal: 20, gap: 14 },
  similarItem: { width: 88, alignItems: "center", gap: 6 },
  similarName: { color: palette.secondary, fontSize: 12 },
});
