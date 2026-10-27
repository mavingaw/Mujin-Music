import {
  HISTORY_PERIODS,
  formatListeningTime,
  getHistoryStats,
  getHistoryTimeline,
  historyPeriod,
  historyTimeLabel,
  localTimezoneOffset,
  type HistoryEntry,
  type HistoryPeriodPreset,
} from "@parson/music-sdk";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { History as HistoryIcon, Radio } from "lucide-react-native";
import { useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  SectionList,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import {
  BrowseHeader,
  Chip,
  browseStyles as styles,
} from "@/components/browse-screen";
import { Screen } from "@/components/music-ui";
import { palette } from "@/constants/colors";
import { useZonePlayback } from "@/hooks/use-zone-playback";
import { historySections } from "@/lib/browse-collections";
import { playableCatalogSong } from "@/lib/playable-song";
import { immediatePressFeedback } from "@/lib/press-feedback";

const PERIODS = HISTORY_PERIODS.filter(
  (option) => option.value !== "last-year",
);

function playable(entry: HistoryEntry) {
  return playableCatalogSong({
    id: entry.songId,
    name: entry.title,
    artistId: entry.artistId ?? "",
    artistName: entry.artistName,
    albumId: entry.albumId ?? "",
    albumName: entry.albumName ?? "",
    coverPath: entry.coverPath ?? "",
    duration: entry.durationSeconds,
  });
}

/** Listening history: a summary of the period and every play by day. */
export default function HistoryScreen() {
  const router = useRouter();
  const playback = useZonePlayback();
  const [preset, setPreset] = useState<HistoryPeriodPreset>("month");
  const period = useMemo(() => historyPeriod(preset), [preset]);
  const filter = useMemo(
    () => ({ from: period.from, to: period.to, tz: localTimezoneOffset() }),
    [period],
  );
  const stats = useQuery({
    queryKey: ["history", "stats", filter],
    queryFn: () => getHistoryStats(filter, { bucket: period.bucket, top: 8 }),
  });
  const timeline = useInfiniteQuery({
    queryKey: ["history", "timeline", filter],
    queryFn: ({ pageParam }) =>
      getHistoryTimeline(filter, { beforeId: pageParam, limit: 80 }),
    initialPageParam: null as number | null,
    getNextPageParam: (page) => page.nextBeforeId ?? undefined,
  });
  const entries = useMemo(
    () => timeline.data?.pages.flatMap((page) => page.items) ?? [],
    [timeline.data],
  );
  const sections = useMemo(() => historySections(entries), [entries]);
  const totals = stats.data?.totals;

  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <SectionList
          contentContainerStyle={{ paddingBottom: 140 }}
          keyExtractor={(entry) => String(entry.id)}
          ListEmptyComponent={
            timeline.isPending ? (
              <ActivityIndicator color="white" style={{ marginTop: 40 }} />
            ) : (
              <View style={styles.empty}>
                <HistoryIcon color={palette.accent} size={36} />
                <Text style={[styles.emptyTitle, { marginTop: 14 }]}>
                  No plays in this period
                </Text>
                <Text style={styles.emptyText}>
                  A track joins your history once you have heard half of it.
                </Text>
              </View>
            )
          }
          ListFooterComponent={
            timeline.hasNextPage ? (
              <Pressable
                accessibilityRole="button"
                onPress={() => void timeline.fetchNextPage()}
                style={local.more}
              >
                {timeline.isFetchingNextPage ? (
                  <ActivityIndicator color="white" />
                ) : (
                  <Text style={local.moreText}>Load earlier plays</Text>
                )}
              </Pressable>
            ) : null
          }
          ListHeaderComponent={
            <>
              <BrowseHeader
                accent={palette.accent}
                kicker="Browse"
                subtitle="Every play on every zone, just for your profile."
                title="History"
              />
              <ScrollView
                contentContainerStyle={styles.chips}
                horizontal
                showsHorizontalScrollIndicator={false}
              >
                {PERIODS.map((option) => (
                  <Chip
                    active={preset === option.value}
                    key={option.value}
                    label={option.label}
                    onPress={() => setPreset(option.value)}
                  />
                ))}
              </ScrollView>
              {totals ? (
                <View style={local.tiles}>
                  {[
                    ["Plays", totals.plays.toLocaleString()],
                    ["Listening", formatListeningTime(totals.listenedSeconds)],
                    ["Artists", String(totals.artists)],
                  ].map(([label, value]) => (
                    <View key={label} style={local.tile}>
                      <Text style={local.tileLabel}>{label}</Text>
                      <Text style={local.tileValue}>{value}</Text>
                    </View>
                  ))}
                </View>
              ) : null}
              {stats.data?.topArtists.length ? (
                <>
                  <View style={styles.sectionHeader}>
                    <Text style={styles.sectionTitle}>Top artists</Text>
                  </View>
                  <ScrollView
                    contentContainerStyle={{ gap: 14, paddingHorizontal: 20 }}
                    horizontal
                    showsHorizontalScrollIndicator={false}
                  >
                    {stats.data.topArtists.map((artist, index) => (
                      <Pressable
                        {...immediatePressFeedback}
                        accessibilityRole="link"
                        disabled={!artist.available}
                        key={artist.id}
                        onPress={() => router.push(`/artist/${artist.id}`)}
                        style={{ width: 92, alignItems: "center" }}
                      >
                        <Artwork
                          path={artist.imagePath}
                          rounded={42}
                          size={84}
                        />
                        <Text numberOfLines={1} style={local.rankName}>
                          {index + 1}. {artist.name}
                        </Text>
                        <Text style={local.rankMeta}>{artist.plays} plays</Text>
                      </Pressable>
                    ))}
                  </ScrollView>
                </>
              ) : null}
            </>
          }
          renderItem={({ item: entry, section }) => (
            <Pressable
              {...immediatePressFeedback}
              accessibilityRole="button"
              disabled={!entry.available}
              onPress={() => {
                const queue = section.data
                  .filter((item) => item.available)
                  .map(playable);
                const song = queue.find((item) => item.id === entry.songId);
                if (song) playback.playSong(song, queue);
              }}
              style={styles.row}
            >
              <Text style={local.time}>
                {historyTimeLabel(entry.listenedAt)}
              </Text>
              <Artwork path={entry.coverPath} size={48} />
              <View style={styles.rowText}>
                <Text numberOfLines={1} style={styles.rowTitle}>
                  {entry.title}
                </Text>
                <Text numberOfLines={1} style={styles.rowMeta}>
                  {[entry.artistName, entry.zoneName]
                    .filter(Boolean)
                    .join(" · ")}
                </Text>
              </View>
              {entry.source === "generated" ? (
                <Radio color={palette.accent} size={16} />
              ) : null}
            </Pressable>
          )}
          renderSectionHeader={({ section }) => (
            <View style={styles.sectionHeader}>
              <Text style={styles.sectionTitle}>{section.title}</Text>
              <Text style={styles.sectionMeta}>{section.subtitle}</Text>
            </View>
          )}
          sections={sections}
          stickySectionHeadersEnabled
        />
      </SafeAreaView>
    </Screen>
  );
}

const local = StyleSheet.create({
  tiles: { flexDirection: "row", gap: 8, paddingHorizontal: 20 },
  tile: {
    flex: 1,
    padding: 12,
    borderRadius: 14,
    backgroundColor: palette.elevated,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.border,
  },
  tileLabel: {
    color: palette.muted,
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1,
    textTransform: "uppercase",
  },
  tileValue: { color: "white", fontSize: 18, fontWeight: "800", marginTop: 4 },
  rankName: { color: "white", fontSize: 12, fontWeight: "700", marginTop: 6 },
  rankMeta: { color: palette.muted, fontSize: 11 },
  time: {
    width: 40,
    color: palette.muted,
    fontSize: 12,
    fontVariant: ["tabular-nums"],
  },
  more: {
    alignSelf: "center",
    marginTop: 16,
    height: 42,
    paddingHorizontal: 20,
    borderRadius: 21,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.borderStrong,
    justifyContent: "center",
  },
  moreText: { color: "white", fontWeight: "700" },
});
