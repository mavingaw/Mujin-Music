import {
  clearedFocus,
  composerFacts,
  composerScope,
  focusCountLabel,
  getFocusCompositions,
  getFocusPage,
  getFocusPerson,
  isFocused,
  scopedFocus,
  withEntity,
  type FocusQuery,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
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
import { CompositionItem } from "@/components/composition-item";
import { FocusSheet } from "@/components/focus-sheet";
import { LibraryFocusBar } from "@/components/library-focus-bar";
import { Screen } from "@/components/music-ui";
import { palette } from "@/constants/colors";
import { immediateBorderlessPressFeedback } from "@/lib/press-feedback";

type Tab = "discography" | "compositions";

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
    // Compositions are focused by work facts: form, instrumentation, period.
    setFocus(
      withEntity(query, next === "compositions" ? "compositions" : "albums"),
    );
  };
  if (person.isPending)
    return (
      <Screen>
        <ActivityIndicator color={palette.text} style={{ flex: 1 }} />
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
            <ArrowLeft color={palette.text} size={25} />
          </Pressable>
        </View>
        <View style={styles.hero}>
          <Text style={styles.kicker}>COMPOSER</Text>
          <Text style={styles.title}>{person.data.name}</Text>
          {person.data.composer && composerFacts(person.data.composer) ? (
            <Text style={styles.facts}>
              {composerFacts(person.data.composer)}
            </Text>
          ) : null}
          {person.data.composer ? (
            <Text style={styles.summary}>
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
          <ActivityIndicator color={palette.text} style={{ marginTop: 40 }} />
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
                  composerId={id}
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
    color: palette.text,
    fontSize: 32,
    fontWeight: "900",
    letterSpacing: -1,
    marginTop: 4,
    marginBottom: 4,
  },
  muted: { color: palette.secondary, textAlign: "center" },
  facts: {
    color: palette.text,
    fontSize: 14,
    fontWeight: "600",
    marginBottom: 2,
  },
  summary: { color: palette.secondary },
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
  tabActive: { backgroundColor: palette.text },
  tabText: { color: palette.text, fontWeight: "700" },
  tabTextActive: { color: palette.background },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    paddingHorizontal: 20,
    paddingTop: 8,
    gap: 16,
  },
  card: { width: 156 },
  albumName: {
    color: palette.text,
    fontWeight: "700",
    marginTop: 8,
    lineHeight: 18,
  },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 2 },
});
