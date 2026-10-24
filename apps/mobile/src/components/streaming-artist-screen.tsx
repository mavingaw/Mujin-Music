import { getStreamingArtist } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
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
import { Screen, SectionTitle } from "@/components/music-ui";
import {
  SourceTag,
  StreamingAlbumRow,
  StreamingTrackRow,
} from "@/components/streaming-ui";
import { palette } from "@/constants/colors";

export function StreamingArtistScreen({ id }: { id: string }) {
  const router = useRouter();
  const artist = useQuery({
    queryKey: ["streaming", "artist", id],
    queryFn: () => getStreamingArtist(id),
    retry: 1,
  });
  if (artist.isPending)
    return (
      <Screen>
        <ActivityIndicator color="white" style={{ flex: 1 }} />
      </Screen>
    );
  const data = artist.data;
  if (!data)
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          onPress={() => void artist.refetch()}
          style={styles.errorState}
        >
          <Text style={styles.error}>Could not load artist · Tap to retry</Text>
        </Pressable>
      </Screen>
    );
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
            <ArrowLeft color="white" size={25} />
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={{ paddingBottom: 135 }}>
          <View style={styles.hero}>
            <Artwork path={data.picture_url} rounded={90} size={180} />
            <SourceTag provider={data.provider} />
            <Text style={styles.name}>{data.name}</Text>
          </View>
          {data.top_tracks.length ? (
            <SectionTitle>Top tracks</SectionTitle>
          ) : null}
          {data.top_tracks.map((track) => (
            <StreamingTrackRow
              key={track.id}
              queue={data.top_tracks}
              track={track}
            />
          ))}
          {data.albums.length ? <SectionTitle>Albums</SectionTitle> : null}
          {data.albums.map((album) => (
            <StreamingAlbumRow album={album} key={album.id} />
          ))}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: { height: 50, paddingHorizontal: 20, justifyContent: "center" },
  hero: { alignItems: "center", gap: 12, paddingVertical: 12 },
  name: {
    color: "white",
    fontSize: 30,
    fontWeight: "900",
    textAlign: "center",
  },
  error: { color: palette.secondary, margin: 30 },
  errorState: { flex: 1, alignItems: "center", justifyContent: "center" },
});
