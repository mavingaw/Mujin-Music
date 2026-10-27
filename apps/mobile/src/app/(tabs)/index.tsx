import { getHomeEssentials } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { ChevronRight, Radio } from "lucide-react-native";
import {
  ActivityIndicator,
  RefreshControl,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import {
  AlbumRail,
  PageTitle,
  Screen,
  SectionTitle,
  SongRow,
} from "@/components/music-ui";
import { layout, palette } from "@/constants/colors";

function LiveRadioEntry() {
  const router = useRouter();
  return (
    <Pressable
      accessibilityLabel="Live Radio"
      accessibilityRole="button"
      onPress={() => router.push("/radio")}
      style={styles.radioEntry}
    >
      <View style={styles.radioIcon}>
        <Radio color="white" size={22} />
      </View>
      <View style={{ flex: 1 }}>
        <Text style={styles.radioTitle}>Live Radio</Text>
        <Text style={styles.radioSubtitle}>
          Stations worldwide, lossless included
        </Text>
      </View>
      <ChevronRight color={palette.secondary} size={20} />
    </Pressable>
  );
}

export default function HomeScreen() {
  const home = useQuery({
    queryKey: ["home"],
    queryFn: getHomeEssentials,
  });
  const recommendedSongs = home.data
    ? home.data.recommended.length
      ? home.data.recommended
      : home.data.shuffle
    : [];
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <ScrollView
          refreshControl={
            <RefreshControl
              tintColor="white"
              refreshing={home.isRefetching}
              onRefresh={() => void home.refetch()}
            />
          }
          contentContainerStyle={{
            paddingBottom: layout.tabBar + layout.miniPlayer + 34,
          }}
        >
          <PageTitle>Home</PageTitle>
          <LiveRadioEntry />
          {home.isPending ? (
            <ActivityIndicator color="white" style={{ marginTop: 70 }} />
          ) : home.error ? (
            <Pressable
              accessibilityRole="button"
              style={styles.errorState}
              onPress={() => void home.refetch()}
            >
              <Text style={styles.error}>Could not load your music.</Text>
              <Text style={styles.retry}>Tap to try again</Text>
            </Pressable>
          ) : home.data ? (
            <>
              <SectionTitle>Recently played</SectionTitle>
              {home.data.continue_listening.slice(0, 8).map((song) => (
                <SongRow
                  key={song.id}
                  song={song}
                  queue={home.data.continue_listening}
                  showAlbum={false}
                />
              ))}
              <SectionTitle>Recommended songs</SectionTitle>
              {recommendedSongs.slice(0, 10).map((song) => (
                <SongRow key={song.id} song={song} queue={recommendedSongs} />
              ))}
              <SectionTitle>Albums you might like</SectionTitle>
              <AlbumRail albums={home.data.albums.slice(0, 6)} />
            </>
          ) : null}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  radioEntry: {
    alignItems: "center",
    backgroundColor: "rgba(244,63,94,0.12)",
    borderColor: "rgba(244,63,94,0.25)",
    borderRadius: layout.radius,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    gap: 12,
    marginBottom: 6,
    marginHorizontal: layout.horizontal,
    padding: 14,
  },
  radioIcon: {
    alignItems: "center",
    backgroundColor: "#e11d48",
    borderRadius: 12,
    height: 42,
    justifyContent: "center",
    width: 42,
  },
  radioTitle: { color: palette.text, fontSize: 16, fontWeight: "700" },
  radioSubtitle: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  error: { color: palette.secondary, textAlign: "center", marginTop: 80 },
  errorState: { alignItems: "center" },
  retry: { color: "white", fontWeight: "700", marginTop: 10 },
});
