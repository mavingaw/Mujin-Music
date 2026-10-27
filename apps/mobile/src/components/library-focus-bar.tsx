import {
  criterionLabel,
  cycleFilter,
  filterKey,
  filterState,
  labelCarriesExclusion,
  removeFilter,
  toggleExclude,
  type FocusQuery,
} from "@parson/music-sdk";
import { Heart, SlidersHorizontal, X } from "lucide-react-native";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";

/** Focus button, liked toggle and active filter chips for a library browser. */
export function LibraryFocusBar({
  countLabel,
  onOpen,
  query,
  setQuery,
}: {
  countLabel: string | null;
  onOpen: () => void;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const liked = filterState(query, { facet: "liked" });
  const chips = query.filters.filter((filter) => filter.facet !== "liked");
  return (
    <View style={styles.container}>
      <ScrollView
        contentContainerStyle={styles.row}
        horizontal
        keyboardShouldPersistTaps="handled"
        showsHorizontalScrollIndicator={false}
      >
        <Pressable
          accessibilityHint="Opens genres, years, formats, credits and more"
          accessibilityLabel={`Focus${query.filters.length ? `, ${query.filters.length} filters` : ""}`}
          accessibilityRole="button"
          onPress={onOpen}
          style={[styles.focus, query.filters.length > 0 && styles.focusActive]}
        >
          <SlidersHorizontal
            color={query.filters.length ? "black" : "white"}
            size={16}
          />
          <Text
            style={[
              styles.focusText,
              query.filters.length > 0 && styles.focusActiveText,
            ]}
          >
            Focus{query.filters.length ? ` · ${query.filters.length}` : ""}
          </Text>
        </Pressable>
        <Pressable
          accessibilityLabel={
            liked === "include"
              ? "Liked only. Double tap to hide liked"
              : liked === "exclude"
                ? "Hiding liked. Double tap to clear"
                : "Show liked only"
          }
          accessibilityRole="button"
          accessibilityState={{ selected: liked !== null }}
          onPress={() => setQuery(cycleFilter(query, { facet: "liked" }))}
          style={[
            styles.heart,
            liked === "include" && styles.heartOn,
            liked === "exclude" && styles.heartOff,
          ]}
        >
          <Heart
            color={liked === "include" ? "#f9b3af" : "white"}
            fill={liked === "include" ? "#f9b3af" : "transparent"}
            size={16}
          />
        </Pressable>
        {chips.map((filter) => {
          const key = filterKey(filter);
          const label = criterionLabel(filter);
          const excluded = Boolean(filter.exclude);
          return (
            <View
              key={key}
              style={[styles.chip, excluded && styles.chipExcluded]}
            >
              <Pressable
                accessibilityLabel={
                  excluded ? `${label}, excluded` : `${label}, focused`
                }
                accessibilityHint={
                  labelCarriesExclusion(filter)
                    ? undefined
                    : "Double tap to switch between include and exclude"
                }
                accessibilityRole="button"
                disabled={labelCarriesExclusion(filter)}
                onPress={() => setQuery(toggleExclude(query, key))}
              >
                <Text
                  numberOfLines={1}
                  style={[
                    styles.chipText,
                    excluded &&
                      !labelCarriesExclusion(filter) &&
                      styles.chipExcludedText,
                  ]}
                >
                  {labelCarriesExclusion(filter) ? "" : excluded ? "− " : "+ "}
                  {label}
                </Text>
              </Pressable>
              <Pressable
                accessibilityLabel={`Remove ${label}`}
                accessibilityRole="button"
                hitSlop={8}
                onPress={() => setQuery(removeFilter(query, key))}
              >
                <X color={palette.secondary} size={15} />
              </Pressable>
            </View>
          );
        })}
      </ScrollView>
      {countLabel ? (
        <Text accessibilityLiveRegion="polite" style={styles.count}>
          {countLabel}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { paddingBottom: 6 },
  row: { flexDirection: "row", gap: 8, paddingHorizontal: 20 },
  focus: {
    height: 36,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    paddingHorizontal: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
  },
  focusActive: { backgroundColor: "white", borderColor: "white" },
  focusText: { color: "white", fontWeight: "700", fontSize: 13 },
  focusActiveText: { color: "black" },
  heart: {
    width: 36,
    height: 36,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  heartOn: {
    borderColor: "rgba(249,179,175,.6)",
    backgroundColor: "rgba(242,71,63,.18)",
  },
  heartOff: { opacity: 0.5 },
  chip: {
    height: 36,
    maxWidth: 240,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    backgroundColor: "rgba(255,255,255,.07)",
    paddingLeft: 13,
    paddingRight: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  chipExcluded: {
    borderColor: "rgba(246,128,122,.45)",
    backgroundColor: "rgba(242,71,63,.1)",
  },
  chipText: { color: "white", fontSize: 13, fontWeight: "600" },
  chipExcludedText: { color: "#fbdcda", textDecorationLine: "line-through" },
  count: {
    color: palette.secondary,
    fontSize: 12,
    marginTop: 8,
    paddingHorizontal: 20,
  },
});
