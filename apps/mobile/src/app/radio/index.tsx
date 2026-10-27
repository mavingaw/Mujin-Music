import {
  getLiveRadioHome,
  searchLiveStations,
  type LiveCategory,
  type LiveSearch,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { ChevronLeft, Search as SearchIcon, X } from "lucide-react-native";
import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { StationListRow, StationRail } from "@/components/live-radio-ui";
import { PageTitle, Screen, SectionTitle } from "@/components/music-ui";
import { layout, palette } from "@/constants/colors";

function categorySearch(category: LiveCategory): LiveSearch {
  return {
    tag: category.tag,
    lossless: category.lossless || undefined,
    min_bitrate: category.min_bitrate,
    order: category.order,
  };
}

/** Live Radio: the community directory, by genre, quality and search. */
export default function LiveRadioScreen() {
  const router = useRouter();
  const [text, setText] = useState("");
  const [name, setName] = useState("");
  const [category, setCategory] = useState<LiveCategory | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setName(text.trim()), 350);
    return () => clearTimeout(timer);
  }, [text]);
  const home = useQuery({
    queryKey: ["live-radio", "home", null],
    queryFn: () => getLiveRadioHome(null),
    staleTime: 5 * 60_000,
  });
  const search: LiveSearch | null =
    name || category
      ? {
          ...(category ? categorySearch(category) : {}),
          name: name || undefined,
          limit: 60,
        }
      : null;
  const results = useQuery({
    queryKey: ["live-radio", "search", search],
    queryFn: () => searchLiveStations(search!),
    enabled: Boolean(search),
    staleTime: 5 * 60_000,
  });
  const data = home.data;
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <ScrollView
          contentContainerStyle={{ paddingBottom: layout.miniPlayer + 48 }}
          keyboardShouldPersistTaps="handled"
          refreshControl={
            <RefreshControl
              onRefresh={() => void home.refetch()}
              refreshing={home.isRefetching}
              tintColor="white"
            />
          }
        >
          <Pressable
            accessibilityLabel="Back"
            accessibilityRole="button"
            hitSlop={10}
            onPress={() => router.back()}
            style={styles.back}
          >
            <ChevronLeft color="white" size={26} />
          </Pressable>
          <PageTitle subtitle="Stations from around the world, through your Mujin Music server">
            Live Radio
          </PageTitle>
          <View style={styles.searchBox}>
            <SearchIcon color={palette.secondary} size={20} />
            <TextInput
              accessibilityLabel="Search stations"
              autoCapitalize="none"
              autoCorrect={false}
              onChangeText={setText}
              placeholder="Search stations"
              placeholderTextColor={palette.secondary}
              style={styles.input}
              value={text}
            />
            {text ? (
              <Pressable
                accessibilityLabel="Clear search"
                accessibilityRole="button"
                onPress={() => setText("")}
              >
                <X color="white" size={18} />
              </Pressable>
            ) : null}
          </View>
          {data?.categories.length ? (
            <ScrollView
              contentContainerStyle={styles.chips}
              horizontal
              showsHorizontalScrollIndicator={false}
            >
              {data.categories.map((item) => {
                const selected = category?.id === item.id;
                return (
                  <Pressable
                    accessibilityRole="button"
                    accessibilityState={{ selected }}
                    key={item.id}
                    onPress={() => setCategory(selected ? null : item)}
                    style={[
                      styles.chip,
                      { backgroundColor: selected ? "white" : item.colors[1] },
                    ]}
                  >
                    <Text
                      style={[styles.chipText, selected && { color: "black" }]}
                    >
                      {item.label}
                    </Text>
                  </Pressable>
                );
              })}
            </ScrollView>
          ) : null}
          {search ? (
            results.isPending ? (
              <ActivityIndicator color="white" style={{ marginTop: 40 }} />
            ) : results.data?.items.length ? (
              <View style={{ marginTop: 8 }}>
                {results.data.items.map((station) => (
                  <StationListRow key={station.id} station={station} />
                ))}
              </View>
            ) : (
              <Text style={styles.empty}>
                {results.isError
                  ? "The radio directory is unreachable right now."
                  : "No stations found."}
              </Text>
            )
          ) : home.isPending ? (
            <ActivityIndicator color="white" style={{ marginTop: 60 }} />
          ) : data ? (
            <>
              {data.directory_error ? (
                <Text style={styles.notice}>{data.directory_error}</Text>
              ) : null}
              {data.my.length ? (
                <>
                  <SectionTitle>My Live Radio</SectionTitle>
                  <StationRail stations={data.my} />
                </>
              ) : null}
              {data.recent.length ? (
                <>
                  <SectionTitle>Recently played</SectionTitle>
                  <StationRail stations={data.recent} />
                </>
              ) : null}
              {data.popular.length ? (
                <>
                  <SectionTitle>Popular now</SectionTitle>
                  <StationRail stations={data.popular} />
                </>
              ) : null}
              {data.lossless.length ? (
                <>
                  <SectionTitle>Lossless stations</SectionTitle>
                  <StationRail stations={data.lossless} />
                </>
              ) : null}
              {data.trending.length ? (
                <>
                  <SectionTitle>Trending</SectionTitle>
                  <StationRail stations={data.trending} />
                </>
              ) : null}
              <Text style={styles.credit}>
                Directory by the community at radio-browser.info. Long-press a
                station for details.
              </Text>
            </>
          ) : (
            <Text style={styles.empty}>
              Live radio is unavailable right now.
            </Text>
          )}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  back: { marginLeft: 12, marginTop: 4, width: 40 },
  searchBox: {
    alignItems: "center",
    backgroundColor: palette.elevatedStrong,
    borderRadius: 14,
    flexDirection: "row",
    gap: 10,
    height: 46,
    marginHorizontal: layout.horizontal,
    marginTop: 4,
    paddingHorizontal: 14,
  },
  input: { color: "white", flex: 1, fontSize: 16 },
  chips: { gap: 8, paddingHorizontal: layout.horizontal, paddingVertical: 14 },
  chip: { borderRadius: 999, paddingHorizontal: 14, paddingVertical: 8 },
  chipText: { color: "white", fontSize: 13, fontWeight: "700" },
  empty: { color: palette.secondary, marginTop: 40, textAlign: "center" },
  notice: {
    color: "#fde68a",
    fontSize: 13,
    marginHorizontal: layout.horizontal,
    marginTop: 8,
  },
  credit: {
    color: palette.muted,
    fontSize: 12,
    lineHeight: 18,
    marginHorizontal: layout.horizontal,
    marginTop: 28,
  },
});
