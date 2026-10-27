import {
  getHomeEssentials,
  getProfiles,
  greetingForHour,
} from "@parson/music-sdk";
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
import {
  ArtistCircles,
  BestYearsCard,
  DailyPicksCard,
  GenreTiles,
  HomeAlbumRail,
  HomeGreeting,
  HomeSectionHeader,
} from "@/components/home-sections";
import { layout, palette } from "@/constants/colors";
import { useSession } from "@/providers/session-provider";

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
        <Radio color={palette.accentText} size={22} />
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

function firstName(name: string) {
  const first = name.trim().split(/\s+/)[0] ?? "";
  return first ? first.charAt(0).toLocaleUpperCase() + first.slice(1) : "";
}

export default function HomeScreen() {
  const session = useSession();
  const profiles = useQuery({
    queryKey: ["profiles", session.claims?.sub],
    queryFn: getProfiles,
    staleTime: 60_000,
  });
  const profileName =
    profiles.data?.profiles.find(
      (profile) => profile.id === profiles.data?.currentId,
    )?.name ??
    session.claims?.username ??
    "";
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
          {home.data ? (
            <HomeGreeting
              feed={home.data}
              greeting={greetingForHour(new Date().getHours())}
              name={firstName(profileName)}
            />
          ) : (
            <PageTitle>Home</PageTitle>
          )}
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
              {home.data.daily_picks?.picks.length ? (
                <DailyPicksCard
                  date={home.data.daily_picks.date}
                  picks={home.data.daily_picks.picks}
                />
              ) : null}
              {home.data.recently_added?.length ? (
                <>
                  <HomeSectionHeader
                    eyebrow="Your library"
                    title="Recently added"
                  />
                  <HomeAlbumRail albums={home.data.recently_added} />
                </>
              ) : null}
              <LiveRadioEntry />
              <SectionTitle>Recently played</SectionTitle>
              {home.data.continue_listening.slice(0, 8).map((song) => (
                <SongRow
                  key={song.id}
                  song={song}
                  queue={home.data.continue_listening}
                  showAlbum={false}
                />
              ))}
              {home.data.new_for_you?.length ? (
                <>
                  <HomeSectionHeader
                    eyebrow="Newest releases you own"
                    title="New for you"
                  />
                  <HomeAlbumRail albums={home.data.new_for_you} />
                </>
              ) : null}
              {home.data.similar_artists?.length ? (
                <>
                  <HomeSectionHeader
                    eyebrow={
                      home.data.similar_artists.find((artist) => artist.reason)
                        ?.reason ?? "From your library"
                    }
                    title="Similar artists"
                  />
                  <ArtistCircles artists={home.data.similar_artists} />
                </>
              ) : null}
              {home.data.best_years?.albums.length ? (
                <BestYearsCard prime={home.data.best_years} />
              ) : null}
              <SectionTitle>Recommended songs</SectionTitle>
              {recommendedSongs.slice(0, 10).map((song) => (
                <SongRow key={song.id} song={song} queue={recommendedSongs} />
              ))}
              {home.data.genres?.length ? (
                <>
                  <HomeSectionHeader eyebrow="Browse" title="Genres" />
                  <GenreTiles genres={home.data.genres} />
                </>
              ) : null}
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
    backgroundColor: "rgba(217,139,95,0.10)",
    borderColor: "rgba(217,139,95,0.25)",
    borderRadius: layout.radius,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    gap: 12,
    marginBottom: 6,
    marginHorizontal: layout.horizontal,
    marginTop: 24,
    padding: 14,
  },
  radioIcon: {
    alignItems: "center",
    backgroundColor: palette.accent,
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
