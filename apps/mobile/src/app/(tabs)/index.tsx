import {
  getHomeEssentials,
  getProfiles,
  greetingForHour,
  visibleHomeRows,
  type HomeEssentials,
  type HomeRowId,
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
import { alpha, layout, palette, storedAppearance } from "@/constants/colors";
import { Fragment, type ReactNode } from "react";
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

/** One Home row, in the order the profile chose (Settings → Appearance). */
function homeRow(
  row: HomeRowId,
  data: HomeEssentials,
  recommendedSongs: HomeEssentials["recommended"],
): ReactNode {
  switch (row) {
    case "daily-picks":
      return data.daily_picks?.picks.length ? (
        <DailyPicksCard
          date={data.daily_picks.date}
          picks={data.daily_picks.picks}
        />
      ) : null;
    case "recently-added":
      return data.recently_added?.length ? (
        <>
          <HomeSectionHeader eyebrow="Your library" title="Recently added" />
          <HomeAlbumRail albums={data.recently_added} />
        </>
      ) : null;
    case "live-radio":
      return <LiveRadioEntry />;
    case "recently-played":
      return data.continue_listening.length ? (
        <>
          <SectionTitle>Recently played</SectionTitle>
          {data.continue_listening.slice(0, 8).map((song) => (
            <SongRow
              key={song.id}
              song={song}
              queue={data.continue_listening}
              showAlbum={false}
            />
          ))}
        </>
      ) : null;
    case "new-for-you":
      return data.new_for_you?.length ? (
        <>
          <HomeSectionHeader
            eyebrow="Newest releases you own"
            title="New for you"
          />
          <HomeAlbumRail albums={data.new_for_you} />
        </>
      ) : null;
    case "similar-artists":
      return data.similar_artists?.length ? (
        <>
          <HomeSectionHeader
            eyebrow={
              data.similar_artists.find((artist) => artist.reason)?.reason ??
              "From your library"
            }
            title="Similar artists"
          />
          <ArtistCircles artists={data.similar_artists} />
        </>
      ) : null;
    case "best-years":
      return data.best_years?.albums.length ? (
        <BestYearsCard prime={data.best_years} />
      ) : null;
    case "recommended-songs":
      return recommendedSongs.length ? (
        <>
          <SectionTitle>Recommended songs</SectionTitle>
          {recommendedSongs.slice(0, 10).map((song) => (
            <SongRow key={song.id} song={song} queue={recommendedSongs} />
          ))}
        </>
      ) : null;
    case "genres":
      return data.genres?.length ? (
        <>
          <HomeSectionHeader eyebrow="Browse" title="Genres" />
          <GenreTiles genres={data.genres} />
        </>
      ) : null;
    case "albums-you-might-like":
      return data.albums.length ? (
        <>
          <SectionTitle>Albums you might like</SectionTitle>
          <AlbumRail albums={data.albums.slice(0, 6)} />
        </>
      ) : null;
    default:
      return null;
  }
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
              tintColor={palette.text}
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
            <ActivityIndicator color={palette.text} style={{ marginTop: 70 }} />
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
              {visibleHomeRows(storedAppearance.appearance).map((row) => (
                <Fragment key={row}>
                  {homeRow(row, home.data!, recommendedSongs)}
                </Fragment>
              ))}
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
    backgroundColor: alpha(palette.accent, 0.1),
    borderColor: alpha(palette.accent, 0.25),
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
  retry: { color: palette.text, fontWeight: "700", marginTop: 10 },
});
