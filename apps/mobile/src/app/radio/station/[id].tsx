import {
  getLiveNowPlaying,
  getLiveStation,
  LIVE_QUALITY_DESCRIPTIONS,
  LIVE_TITLE_POLL_MS,
  liveCountryName,
  liveFormatText,
  liveTitleText,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ChevronLeft, Heart, Pause, Play } from "lucide-react-native";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import {
  LiveTag,
  QualityTag,
  StationLogo,
  StationRail,
  usePlayStation,
  useToggleSaved,
} from "@/components/live-radio-ui";
import { Screen, SectionTitle } from "@/components/music-ui";
import { alpha, layout, palette } from "@/constants/colors";
import { usePlayer } from "@/providers/player-provider";

/** A station: play, save, what it is playing and how it sounds. */
export default function StationScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const player = usePlayer();
  const play = usePlayStation();
  const toggle = useToggleSaved();
  const query = useQuery({
    queryKey: ["live-radio", "station", id],
    queryFn: () => getLiveStation(id!),
    enabled: Boolean(id),
  });
  const station = query.data?.station;
  const current = station ? play.isCurrent(station) : false;
  const live = useQuery({
    queryKey: ["live-radio", "now-playing", id],
    queryFn: async () => (await getLiveNowPlaying([id!]))[id!] ?? null,
    enabled: Boolean(id),
    refetchInterval: current ? LIVE_TITLE_POLL_MS : false,
  });
  const playing = live.data ?? query.data?.now_playing;
  if (!station) {
    return (
      <Screen>
        <SafeAreaView style={{ flex: 1 }}>
          {query.isPending ? (
            <ActivityIndicator color={palette.text} style={{ marginTop: 80 }} />
          ) : (
            <Text style={styles.empty}>This station could not be found.</Text>
          )}
        </SafeAreaView>
      </Screen>
    );
  }
  const quality = playing?.quality ?? station.quality;
  const place = [
    station.region,
    liveCountryName(station.country_code, station.country),
  ]
    .filter((part, index, parts) => part && parts.indexOf(part) === index)
    .join(", ");
  const isPlaying = current && player.isPlaying;
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <ScrollView
          contentContainerStyle={{ paddingBottom: layout.miniPlayer + 48 }}
        >
          <Pressable
            accessibilityLabel="Back"
            accessibilityRole="button"
            hitSlop={10}
            onPress={() => router.back()}
            style={styles.back}
          >
            <ChevronLeft color={palette.text} size={26} />
          </Pressable>
          <View style={styles.header}>
            <StationLogo rounded={20} size={196} station={station} />
            <View style={styles.badges}>
              {isPlaying ? <LiveTag /> : null}
              <QualityTag quality={quality} />
            </View>
            <Text accessibilityRole="header" style={styles.title}>
              {station.name}
            </Text>
            {place ? <Text style={styles.place}>{place}</Text> : null}
            <View style={styles.actions}>
              <Pressable
                accessibilityRole="button"
                onPress={() => play.play(station)}
                style={styles.playButton}
              >
                {isPlaying ? (
                  <Pause
                    color={palette.background}
                    fill={palette.background}
                    size={20}
                  />
                ) : (
                  <Play
                    color={palette.background}
                    fill={palette.background}
                    size={20}
                  />
                )}
                <Text style={styles.playText}>
                  {isPlaying ? "Pause" : current ? "Resume" : "Play now"}
                </Text>
              </Pressable>
              <Pressable
                accessibilityLabel={
                  station.saved
                    ? "Remove from My Live Radio"
                    : "Add to My Live Radio"
                }
                accessibilityRole="button"
                accessibilityState={{ selected: station.saved }}
                disabled={toggle.isPending}
                onPress={() => toggle.mutate(station)}
                style={styles.saveButton}
              >
                <Heart
                  color={station.saved ? palette.danger : palette.text}
                  fill={station.saved ? palette.danger : "transparent"}
                  size={20}
                />
              </Pressable>
            </View>
          </View>
          <SectionTitle>On air</SectionTitle>
          {playing?.title ? (
            <View style={styles.onAir}>
              <Text numberOfLines={2} style={styles.onAirTitle}>
                {playing.title.title}
              </Text>
              {playing.title.artist ? (
                <Text numberOfLines={1} style={styles.onAirArtist}>
                  {playing.title.artist}
                </Text>
              ) : null}
            </View>
          ) : (
            <Text style={styles.body}>
              Play the station to see what is on air. Titles come from the
              stream itself.
            </Text>
          )}
          {playing?.history.slice(0, 6).map((item) => (
            <Text
              key={`${item.at}-${item.raw}`}
              numberOfLines={1}
              style={styles.history}
            >
              {liveTitleText(item)}
            </Text>
          ))}
          <SectionTitle>Sound</SectionTitle>
          <Text style={styles.body}>
            {playing?.format ? liveFormatText(playing.format) : quality.label}
            {playing?.format?.verified ? " · read from the audio" : ""}
          </Text>
          <Text style={styles.note}>
            {LIVE_QUALITY_DESCRIPTIONS[quality.tier]}
          </Text>
          {station.tags.length ? (
            <Text style={styles.tags}>{station.tags.join(" · ")}</Text>
          ) : null}
          {query.data?.similar.length ? (
            <>
              <SectionTitle>More like this</SectionTitle>
              <StationRail stations={query.data.similar} />
            </>
          ) : null}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  back: { marginLeft: 12, marginTop: 4, width: 40 },
  empty: { color: palette.secondary, marginTop: 80, textAlign: "center" },
  header: {
    alignItems: "center",
    paddingHorizontal: layout.horizontal,
    paddingTop: 8,
  },
  badges: { flexDirection: "row", gap: 8, marginTop: 16 },
  title: {
    color: palette.text,
    fontSize: 30,
    fontWeight: "900",
    letterSpacing: -0.5,
    marginTop: 10,
    textAlign: "center",
  },
  place: { color: palette.secondary, fontSize: 14, marginTop: 6 },
  actions: {
    alignItems: "center",
    flexDirection: "row",
    gap: 12,
    marginTop: 20,
  },
  playButton: {
    alignItems: "center",
    backgroundColor: palette.text,
    borderRadius: 999,
    flexDirection: "row",
    gap: 8,
    height: 48,
    paddingHorizontal: 26,
  },
  playText: { color: palette.background, fontSize: 15, fontWeight: "800" },
  saveButton: {
    alignItems: "center",
    borderColor: palette.borderStrong,
    borderRadius: 24,
    borderWidth: 1,
    height: 48,
    justifyContent: "center",
    width: 48,
  },
  onAir: {
    backgroundColor: alpha(palette.danger, 0.12),
    borderRadius: 14,
    marginHorizontal: layout.horizontal,
    padding: 14,
  },
  onAirTitle: { color: palette.text, fontSize: 17, fontWeight: "700" },
  onAirArtist: { color: palette.secondary, fontSize: 14, marginTop: 2 },
  history: {
    color: palette.secondary,
    fontSize: 13,
    marginHorizontal: layout.horizontal,
    marginTop: 8,
  },
  body: { color: "#e9e5dd", fontSize: 14, marginHorizontal: layout.horizontal },
  note: {
    color: palette.muted,
    fontSize: 12,
    lineHeight: 18,
    marginHorizontal: layout.horizontal,
    marginTop: 6,
  },
  tags: {
    color: palette.secondary,
    fontSize: 12,
    marginHorizontal: layout.horizontal,
    marginTop: 14,
    textTransform: "uppercase",
  },
});
