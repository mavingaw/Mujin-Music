import {
  clearedFocus,
  composerScope,
  focusCountLabel,
  getFocusCompositions,
  getFocusPage,
  getFocusPerson,
  isFocused,
  scopedFocus,
  withEntity,
  type Composition,
  type FocusQuery,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft, ChevronDown, Play } from "lucide-react-native";
import { useMemo, useState } from "react";
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
import { FocusSheet } from "@/components/focus-sheet";
import { LibraryFocusBar } from "@/components/library-focus-bar";
import { Screen } from "@/components/music-ui";
import { palette } from "@/constants/colors";
import { recordingCaption, recordingSongs } from "@/lib/library-focus";
import { immediateBorderlessPressFeedback } from "@/lib/press-feedback";
import { usePlayer } from "@/providers/player-provider";

type Tab = "discography" | "compositions";

function durationLabel(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "";
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  return `${minutes}:${rest < 10 ? "0" : ""}${rest}`;
}

export default function ComposerScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const scope = useMemo(() => (id ? [composerScope(id)] : []), [id]);
  const [tab, setTab] = useState<Tab>("discography");
  const [focus, setFocus] = useState<FocusQuery>(() =>
    scopedFocus("albums", scope),
  );
  const [focusOpen, setFocusOpen] = useState(false);
  const query = focus.scope === scope ? focus : scopedFocus("albums", scope);
  const person = useQuery({
    queryKey: ["library", "focus", "person", id],
    queryFn: () => getFocusPerson(id),
    enabled: Boolean(id),
  });
  const albums = useQuery({
    queryKey: ["library", "focus", "page", "mobile-composer", query],
    queryFn: () => getFocusPage(query, 0, 200),
    enabled: Boolean(id) && tab === "discography",
    placeholderData: keepPreviousData,
  });
  const compositions = useQuery({
    queryKey: ["library", "focus", "compositions", "mobile", query],
    queryFn: () => getFocusCompositions(query),
    enabled: Boolean(id) && tab === "compositions",
    placeholderData: keepPreviousData,
  });
  const selectTab = (next: Tab) => {
    setTab(next);
    setFocus(withEntity(query, next === "compositions" ? "songs" : "albums"));
  };
  if (person.isPending)
    return (
      <Screen>
        <ActivityIndicator color="white" style={{ flex: 1 }} />
      </Screen>
    );
  if (!person.data)
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          style={styles.center}
          onPress={() => void person.refetch()}
        >
          <Text style={styles.muted}>
            Could not load composer · Tap to retry
          </Text>
        </Pressable>
      </Screen>
    );
  const count =
    tab === "discography"
      ? focusCountLabel(query, albums.data?.total, albums.data?.libraryTotal)
      : compositions.data
        ? `${compositions.data.total} ${compositions.data.total === 1 ? "composition" : "compositions"}`
        : null;
  const loading =
    (tab === "discography" && albums.isPending) ||
    (tab === "compositions" && compositions.isPending);
  const empty =
    tab === "discography"
      ? albums.data?.albums.length === 0
      : compositions.data?.compositions.length === 0;
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <View style={styles.nav}>
          <Pressable
            {...immediateBorderlessPressFeedback}
            accessibilityLabel="Back"
            accessibilityRole="button"
            hitSlop={12}
            onPress={router.back}
          >
            <ArrowLeft color="white" size={25} />
          </Pressable>
        </View>
        <View style={styles.hero}>
          <Text style={styles.kicker}>COMPOSER</Text>
          <Text style={styles.title}>{person.data.name}</Text>
          {person.data.composer ? (
            <Text style={styles.muted}>
              {person.data.composer.workCount} compositions ·{" "}
              {person.data.composer.albumCount} albums
            </Text>
          ) : null}
        </View>
        <View accessibilityRole="tablist" style={styles.tabs}>
          {(["discography", "compositions"] as const).map((value) => (
            <Pressable
              accessibilityRole="tab"
              accessibilityState={{ selected: tab === value }}
              key={value}
              onPress={() => selectTab(value)}
              style={[styles.tab, tab === value && styles.tabActive]}
            >
              <Text
                style={[styles.tabText, tab === value && styles.tabTextActive]}
              >
                {value === "discography" ? "Discography" : "Compositions"}
              </Text>
            </Pressable>
          ))}
        </View>
        <LibraryFocusBar
          countLabel={count}
          onOpen={() => setFocusOpen(true)}
          query={query}
          setQuery={setFocus}
        />
        {loading ? (
          <ActivityIndicator color="white" style={{ marginTop: 40 }} />
        ) : empty ? (
          <Pressable
            accessibilityRole="button"
            disabled={!isFocused(query)}
            onPress={() => setFocus(clearedFocus(query))}
            style={styles.center}
          >
            <Text style={styles.muted}>
              {isFocused(query)
                ? "Nothing matches this focus · Tap to clear"
                : "Nothing credits this composer yet."}
            </Text>
          </Pressable>
        ) : (
          <ScrollView contentContainerStyle={{ paddingBottom: 150 }}>
            {tab === "discography" ? (
              <View style={styles.grid}>
                {albums.data?.albums.map((album) => (
                  <Pressable
                    accessibilityLabel={`${album.name} by ${album.artistName}`}
                    accessibilityRole="button"
                    key={album.id}
                    onPress={() => router.push(`/album/${album.id}`)}
                    style={styles.card}
                  >
                    <Artwork path={album.coverPath} rounded={9} size={156} />
                    <Text numberOfLines={2} style={styles.albumName}>
                      {album.name}
                    </Text>
                    <Text numberOfLines={1} style={styles.meta}>
                      {[album.artistName, album.releaseYear]
                        .filter(Boolean)
                        .join(" · ")}
                    </Text>
                  </Pressable>
                ))}
              </View>
            ) : (
              compositions.data?.compositions.map((composition) => (
                <CompositionItem
                  composition={composition}
                  key={composition.key}
                />
              ))
            )}
          </ScrollView>
        )}
        <FocusSheet
          bookmarks={false}
          onClose={() => setFocusOpen(false)}
          open={focusOpen}
          query={query}
          setQuery={setFocus}
        />
      </SafeAreaView>
    </Screen>
  );
}

function CompositionItem({ composition }: { composition: Composition }) {
  const player = usePlayer();
  const [open, setOpen] = useState(false);
  const first = composition.recordings[0];
  const play = (index: number) => {
    const recording = composition.recordings[index];
    if (!recording) return;
    const songs = recordingSongs(recording);
    if (songs[0]) player.playSong(songs[0], songs);
  };
  return (
    <View style={styles.composition}>
      <View style={styles.compositionRow}>
        <Pressable
          accessibilityLabel={`Play ${composition.title}`}
          accessibilityRole="button"
          disabled={!first}
          hitSlop={8}
          onPress={() => play(0)}
          style={styles.playSmall}
        >
          <Play color="black" fill="black" size={14} />
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ expanded: open }}
          onPress={() => setOpen((value) => !value)}
          style={styles.compositionMain}
        >
          <View style={{ flex: 1 }}>
            <Text numberOfLines={2} style={styles.name}>
              {composition.title}
            </Text>
            <Text numberOfLines={1} style={styles.meta}>
              {composition.recordings.length}{" "}
              {composition.recordings.length === 1 ? "recording" : "recordings"}
              {composition.movementCount > 1
                ? ` · ${composition.movementCount} movements`
                : ""}
            </Text>
          </View>
          <ChevronDown
            color={palette.secondary}
            size={18}
            style={open ? { transform: [{ rotate: "180deg" }] } : undefined}
          />
        </Pressable>
      </View>
      {open
        ? composition.recordings.map((recording, index) => (
            <Pressable
              accessibilityLabel={`Play ${recording.albumName ?? "recording"}`}
              accessibilityRole="button"
              key={`${recording.albumId ?? "loose"}-${index}`}
              onPress={() => play(index)}
              style={styles.recording}
            >
              <Artwork path={recording.coverPath ?? ""} size={44} />
              <View style={{ flex: 1 }}>
                <Text numberOfLines={1} style={styles.recordingName}>
                  {recording.albumName ?? "Single track"}
                </Text>
                <Text numberOfLines={1} style={styles.meta}>
                  {recordingCaption(recording)}
                </Text>
                {recording.tracks.length > 1
                  ? recording.tracks.map((track, trackIndex) => (
                      <Text
                        key={track.id}
                        numberOfLines={1}
                        style={styles.movement}
                      >
                        {track.movementNumber ?? trackIndex + 1}.{" "}
                        {track.movement ?? track.title}{" "}
                        {durationLabel(track.durationSeconds)}
                      </Text>
                    ))
                  : null}
              </View>
              <Text style={styles.meta}>
                {durationLabel(recording.durationSeconds)}
              </Text>
            </Pressable>
          ))
        : null}
    </View>
  );
}

const styles = StyleSheet.create({
  nav: { height: 48, paddingHorizontal: 20, justifyContent: "center" },
  hero: { paddingHorizontal: 20, paddingBottom: 16 },
  kicker: {
    color: palette.secondary,
    fontSize: 12,
    fontWeight: "700",
    letterSpacing: 1.6,
  },
  title: {
    color: "white",
    fontSize: 32,
    fontWeight: "900",
    letterSpacing: -1,
    marginTop: 4,
    marginBottom: 4,
  },
  muted: { color: palette.secondary, textAlign: "center" },
  center: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
  },
  tabs: {
    flexDirection: "row",
    gap: 8,
    paddingHorizontal: 20,
    marginBottom: 6,
  },
  tab: {
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderRadius: 999,
    backgroundColor: palette.elevated,
  },
  tabActive: { backgroundColor: "white" },
  tabText: { color: "white", fontWeight: "700" },
  tabTextActive: { color: "black" },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    paddingHorizontal: 20,
    paddingTop: 8,
    gap: 16,
  },
  card: { width: 156 },
  albumName: {
    color: "white",
    fontWeight: "700",
    marginTop: 8,
    lineHeight: 18,
  },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 2 },
  composition: {
    borderBottomColor: palette.elevated,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 20,
    paddingVertical: 10,
  },
  compositionRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  compositionMain: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
  },
  playSmall: {
    width: 32,
    height: 32,
    borderRadius: 16,
    backgroundColor: "white",
    alignItems: "center",
    justifyContent: "center",
  },
  name: { color: "white", fontWeight: "700", fontSize: 15 },
  recording: {
    flexDirection: "row",
    gap: 12,
    alignItems: "flex-start",
    paddingVertical: 10,
    paddingLeft: 44,
  },
  recordingName: { color: "white", fontWeight: "600" },
  movement: { color: palette.muted, fontSize: 12, marginTop: 2 },
});
