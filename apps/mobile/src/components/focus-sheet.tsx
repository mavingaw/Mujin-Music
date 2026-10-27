import {
  SORT_OPTIONS,
  createSmartPlaylist,
  cycleFilter,
  decadeCriterion,
  isYearFacet,
  decadesFromYears,
  deleteSavedFocus,
  effectiveDirection,
  effectiveSort,
  facetBaseQuery,
  facetSelectionSummary,
  facetTitle,
  facetValueState,
  filterState,
  focusSummary,
  getFocusFacets,
  getSavedFocuses,
  orderedFacetValues,
  saveFocus,
  toggleFacetValue,
  visibleFacets,
  type Facet,
  type FilterState,
  type FocusQuery,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  Bookmark,
  ChevronDown,
  ListMusic,
  Trash2,
  X,
} from "lucide-react-native";
import { useEffect, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { palette } from "@/constants/colors";
import {
  chipTone,
  facetChipAccessibilityLabel,
  smartPlaylistName,
} from "@/lib/library-focus";

/** Chips a facet shows, most frequent first, before "View more". */
const COLLAPSED_VALUES = 8;
/** Facets open when the sheet opens (plus any with a filter). */
const OPEN_FACETS = 3;
/** Facets with more values than this get a search field. */
const SEARCH_FROM = 20;

export function FocusSheet({
  bookmarks = true,
  onClose,
  open,
  query,
  setQuery,
}: {
  /** Scoped browsers (a composer, an album) have no bookmarks. */
  bookmarks?: boolean;
  onClose: () => void;
  open: boolean;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const client = useQueryClient();
  const base = facetBaseQuery(query);
  const facets = useQuery({
    queryKey: [
      "library",
      "focus",
      "facets",
      base.entity,
      base.match ?? "all",
      base.filters,
      base.text ?? null,
      base.scope ?? [],
      "mobile",
    ],
    queryFn: () => getFocusFacets(base, { limit: 40 }),
    enabled: open,
    placeholderData: (previous) => previous,
  });
  const saved = useQuery({
    queryKey: ["library", "focus", "saved"],
    queryFn: getSavedFocuses,
    enabled: open && bookmarks,
  });
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [bookmarkName, setBookmarkName] = useState("");
  const [busy, setBusy] = useState<"bookmark" | "smart" | null>(null);
  const [message, setMessage] = useState("");
  const columns = visibleFacets(facets.data?.facets ?? [], query.entity, {
    composerScoped: Boolean(
      query.scope?.some(
        (criterion) =>
          criterion.facet === "credit" && criterion.role === "composer",
      ),
    ),
    query,
  });
  const sort = effectiveSort(query);
  const direction = effectiveDirection(query);
  const focused = query.filters.length > 0 || Boolean(query.text?.trim());

  const bookmark = async () => {
    if (!bookmarkName.trim() || busy) return;
    setBusy("bookmark");
    setMessage("");
    try {
      await saveFocus(bookmarkName.trim(), query);
      setBookmarkName("");
      await client.invalidateQueries({
        queryKey: ["library", "focus", "saved"],
      });
      setMessage("Bookmark saved.");
    } catch {
      setMessage("Could not save the bookmark.");
    } finally {
      setBusy(null);
    }
  };

  const smartPlaylist = async () => {
    if (busy) return;
    setBusy("smart");
    setMessage("");
    try {
      const playlist = await createSmartPlaylist(
        smartPlaylistName(query),
        query,
        {
          description: focusSummary(query) || undefined,
        },
      );
      await client.invalidateQueries({ queryKey: ["playlists"] });
      onClose();
      router.push(`/playlist/${playlist.id}`);
    } catch {
      setMessage("Could not create the smart playlist.");
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal
      animationType="slide"
      onRequestClose={onClose}
      presentationStyle="pageSheet"
      transparent={false}
      visible={open}
    >
      <View
        style={[styles.sheet, { paddingTop: Math.max(12, insets.top / 2) }]}
      >
        <View style={styles.header}>
          <Text style={styles.title}>Focus</Text>
          {focused ? (
            <Pressable
              accessibilityRole="button"
              onPress={() =>
                setQuery({
                  ...query,
                  filters: [],
                  text: null,
                  match: undefined,
                })
              }
              style={styles.headerButton}
            >
              <Text style={styles.headerButtonText}>Clear all</Text>
            </Pressable>
          ) : null}
          <Pressable
            accessibilityLabel="Close Focus"
            accessibilityRole="button"
            onPress={onClose}
            style={styles.close}
          >
            <X color="white" size={20} />
          </Pressable>
        </View>
        <ScrollView
          contentContainerStyle={{ paddingBottom: insets.bottom + 40 }}
          keyboardShouldPersistTaps="handled"
        >
          <View style={styles.segment}>
            {(["all", "any"] as const).map((mode) => {
              const active = (query.match ?? "all") === mode;
              return (
                <Pressable
                  accessibilityRole="radio"
                  accessibilityState={{ checked: active }}
                  key={mode}
                  onPress={() =>
                    setQuery({
                      ...query,
                      match: mode === "all" ? undefined : "any",
                    })
                  }
                  style={[styles.segmentItem, active && styles.segmentActive]}
                >
                  <Text
                    style={[
                      styles.segmentText,
                      active && styles.segmentActiveText,
                    ]}
                  >
                    Match {mode}
                  </Text>
                </Pressable>
              );
            })}
          </View>

          <Text style={styles.section}>Sort</Text>
          <View style={styles.wrap}>
            {SORT_OPTIONS[query.entity].map((option) => (
              <Chip
                key={option.value}
                label={option.label}
                onPress={() =>
                  setQuery({
                    ...query,
                    sort: option.value,
                    direction: null,
                    seed:
                      option.value === "random"
                        ? Math.floor(Math.random() * 1_000_000) + 1
                        : null,
                  })
                }
                state={sort === option.value ? "include" : null}
              />
            ))}
            {sort !== "random" ? (
              <Pressable
                accessibilityLabel={
                  direction === "asc" ? "Ascending" : "Descending"
                }
                accessibilityRole="button"
                onPress={() =>
                  setQuery({
                    ...query,
                    direction: direction === "asc" ? "desc" : "asc",
                  })
                }
                style={styles.iconChip}
              >
                {direction === "asc" ? (
                  <ArrowUpNarrowWide color="white" size={16} />
                ) : (
                  <ArrowDownWideNarrow color="white" size={16} />
                )}
              </Pressable>
            ) : null}
          </View>

          {facets.isPending && !facets.data ? (
            <ActivityIndicator color="white" style={{ marginTop: 28 }} />
          ) : null}
          {columns.map((facet, index) => (
            <FacetSection
              expanded={Boolean(expanded[facetId(facet)])}
              facet={facet}
              initiallyOpen={
                index < OPEN_FACETS ||
                facetSelectionSummary(facet, query) !== ""
              }
              key={facetId(facet)}
              onExpand={() =>
                setExpanded((current) => ({
                  ...current,
                  [facetId(facet)]: true,
                }))
              }
              query={query}
              setQuery={setQuery}
            />
          ))}

          {bookmarks ? <Text style={styles.section}>Bookmarks</Text> : null}
          {(bookmarks ? (saved.data ?? []) : []).map((item) => (
            <View key={item.id} style={styles.savedRow}>
              <Pressable
                accessibilityRole="button"
                onPress={() => {
                  setQuery(item.query);
                  onClose();
                }}
                style={styles.savedMain}
              >
                <Bookmark color={palette.secondary} size={18} />
                <View style={{ flex: 1 }}>
                  <Text numberOfLines={1} style={styles.savedName}>
                    {item.name}
                  </Text>
                  <Text numberOfLines={1} style={styles.savedMeta}>
                    {item.view} · {focusSummary(item.query) || "Everything"}
                  </Text>
                </View>
              </Pressable>
              <Pressable
                accessibilityLabel={`Delete ${item.name}`}
                accessibilityRole="button"
                onPress={() =>
                  void deleteSavedFocus(item.id).then(() =>
                    client.invalidateQueries({
                      queryKey: ["library", "focus", "saved"],
                    }),
                  )
                }
                style={styles.savedDelete}
              >
                <Trash2 color={palette.secondary} size={18} />
              </Pressable>
            </View>
          ))}
          <View style={[styles.bookmarkRow, !bookmarks && { display: "none" }]}>
            <TextInput
              accessibilityLabel="Bookmark name"
              maxLength={100}
              onChangeText={setBookmarkName}
              onSubmitEditing={() => void bookmark()}
              placeholder="Bookmark this view"
              placeholderTextColor={palette.muted}
              style={styles.input}
              value={bookmarkName}
            />
            <Pressable
              accessibilityRole="button"
              disabled={!bookmarkName.trim() || busy !== null}
              onPress={() => void bookmark()}
              style={[
                styles.primary,
                (!bookmarkName.trim() || busy !== null) && styles.disabled,
              ]}
            >
              <Text style={styles.primaryText}>
                {busy === "bookmark" ? "Saving…" : "Save"}
              </Text>
            </Pressable>
          </View>
          {focused ? (
            <Pressable
              accessibilityRole="button"
              disabled={busy !== null}
              onPress={() => void smartPlaylist()}
              style={[styles.smart, busy !== null && styles.disabled]}
            >
              <ListMusic color="black" size={18} />
              <Text style={styles.smartText}>
                {busy === "smart" ? "Creating…" : "Create smart playlist"}
              </Text>
            </Pressable>
          ) : null}
          {message ? (
            <Text accessibilityRole="alert" style={styles.message}>
              {message}
            </Text>
          ) : null}
        </ScrollView>
      </View>
    </Modal>
  );
}

function facetId(facet: Facet) {
  return `${facet.key}:${facet.role ?? ""}`;
}

function FacetSection({
  expanded,
  facet,
  initiallyOpen,
  onExpand,
  query,
  setQuery,
}: {
  expanded: boolean;
  facet: Facet;
  initiallyOpen: boolean;
  onExpand: () => void;
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const [search, setSearch] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(search.trim()), 250);
    return () => clearTimeout(timer);
  }, [search]);
  // Long lists search the server, so a large library stays reachable.
  const searchable =
    open && expanded && !isYearFacet(facet.key) && facet.total > SEARCH_FROM;
  const base = facetBaseQuery(query);
  const found = useQuery({
    queryKey: [
      "library",
      "focus",
      "facet-values",
      facet.key,
      facet.role ?? null,
      debounced,
      base.entity,
      base.match ?? "all",
      base.filters,
      base.text ?? null,
      base.scope ?? [],
      "mobile",
    ],
    queryFn: () =>
      getFocusFacets(base, {
        only: [facet.key],
        search: debounced,
        limit: 200,
      }),
    enabled: searchable && debounced.length > 0,
    placeholderData: (previous) => previous,
  });
  const summary = facetSelectionSummary(facet, query);
  const title = facetTitle(facet);

  let body: ReactNode = null;
  if (open && isYearFacet(facet.key)) {
    const yearKey = facet.key;
    body = (
      <View style={styles.wrap}>
        {decadesFromYears(facet.values).map((bucket) => (
          <Chip
            count={bucket.count}
            key={bucket.decade}
            label={`${bucket.decade}s`}
            onPress={() =>
              setQuery(
                cycleFilter(query, decadeCriterion(bucket.decade, yearKey)),
              )
            }
            state={filterState(query, decadeCriterion(bucket.decade, yearKey))}
          />
        ))}
      </View>
    );
  } else if (open) {
    const searched =
      searchable && debounced
        ? (found.data?.facets.find(
            (candidate) =>
              candidate.key === facet.key &&
              (candidate.role ?? null) === (facet.role ?? null),
          ) ?? null)
        : null;
    const { values, more } = orderedFacetValues(searched ?? facet, query, {
      cap: expanded ? undefined : COLLAPSED_VALUES,
    });
    body = (
      <>
        {searchable ? (
          <TextInput
            accessibilityLabel={`Search ${title}`}
            onChangeText={setSearch}
            placeholder={`Search ${facet.total} ${title.toLowerCase()}`}
            placeholderTextColor={palette.muted}
            style={[styles.input, styles.facetSearch]}
            value={search}
          />
        ) : null}
        <View style={styles.wrap}>
          {values.map((value) => (
            <Chip
              count={value.count}
              key={value.value}
              label={value.label}
              onPress={() => setQuery(toggleFacetValue(query, facet, value))}
              state={facetValueState(query, facet, value)}
            />
          ))}
          {searched && values.length === 0 ? (
            <Text style={styles.moreText}>No matches</Text>
          ) : null}
          {!expanded && more > 0 ? (
            <Pressable
              accessibilityRole="button"
              onPress={onExpand}
              style={styles.moreChip}
            >
              <Text style={styles.moreText}>View {more} more</Text>
            </Pressable>
          ) : null}
        </View>
      </>
    );
  }

  return (
    <View style={styles.facet}>
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen((current) => !current)}
        style={styles.facetHeader}
      >
        <View style={{ flex: 1 }}>
          <Text style={styles.facetTitle}>{title}</Text>
          <Text
            numberOfLines={1}
            style={[styles.facetSummary, summary ? styles.facetActive : null]}
          >
            {summary ||
              `${facet.total} ${facet.total === 1 ? "value" : "values"}`}
          </Text>
        </View>
        <ChevronDown
          color={palette.secondary}
          size={18}
          style={open ? { transform: [{ rotate: "180deg" }] } : undefined}
        />
      </Pressable>
      {body ? <View style={styles.facetBody}>{body}</View> : null}
    </View>
  );
}

function Chip({
  count,
  label,
  onPress,
  state,
}: {
  count?: number;
  label: string;
  onPress: () => void;
  state: FilterState;
}) {
  const tone = chipTone(state);
  return (
    <Pressable
      accessibilityLabel={
        count === undefined
          ? label
          : facetChipAccessibilityLabel(label, count, state)
      }
      accessibilityRole="button"
      accessibilityState={{ selected: state !== null }}
      onPress={onPress}
      style={({ pressed }) => [
        styles.chip,
        tone === "include" && styles.chipInclude,
        tone === "exclude" && styles.chipExclude,
        pressed && { opacity: 0.6 },
      ]}
    >
      <Text
        numberOfLines={1}
        style={[
          styles.chipText,
          tone === "include" && styles.chipIncludeText,
          tone === "exclude" && styles.chipExcludeText,
        ]}
      >
        {tone === "include" ? "+ " : tone === "exclude" ? "− " : ""}
        {label}
      </Text>
      {count !== undefined ? (
        <Text
          style={[
            styles.chipCount,
            tone === "include" && styles.chipIncludeCount,
          ]}
        >
          {count}
        </Text>
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  sheet: { flex: 1, backgroundColor: "#0b0b0d", paddingHorizontal: 20 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingBottom: 12,
  },
  title: { flex: 1, color: "white", fontSize: 24, fontWeight: "800" },
  headerButton: { paddingHorizontal: 10, paddingVertical: 8 },
  headerButtonText: { color: palette.secondary, fontWeight: "700" },
  close: {
    width: 40,
    height: 40,
    borderRadius: 20,
    backgroundColor: "rgba(255,255,255,.08)",
    alignItems: "center",
    justifyContent: "center",
  },
  segment: {
    flexDirection: "row",
    alignSelf: "flex-start",
    borderRadius: 20,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    padding: 3,
    marginTop: 4,
  },
  segmentItem: { paddingHorizontal: 14, paddingVertical: 7, borderRadius: 16 },
  segmentActive: { backgroundColor: "white" },
  segmentText: { color: palette.secondary, fontWeight: "700", fontSize: 13 },
  segmentActiveText: { color: "black" },
  section: {
    color: palette.secondary,
    fontSize: 12,
    fontWeight: "800",
    letterSpacing: 0.8,
    textTransform: "uppercase",
    marginTop: 22,
    marginBottom: 10,
  },
  wrap: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  facet: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: palette.borderStrong,
  },
  facetHeader: {
    minHeight: 56,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
  },
  facetTitle: {
    color: palette.secondary,
    fontSize: 12,
    fontWeight: "800",
    letterSpacing: 0.8,
    textTransform: "uppercase",
  },
  facetSummary: { color: palette.muted, fontSize: 14, marginTop: 3 },
  facetActive: { color: "white", fontWeight: "600" },
  facetBody: { paddingBottom: 16 },
  facetSearch: { flex: 0, height: 42, marginBottom: 10 },
  chip: {
    minHeight: 36,
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    paddingHorizontal: 13,
  },
  chipInclude: { backgroundColor: "white", borderColor: "white" },
  chipExclude: {
    borderColor: "rgba(251,113,133,.5)",
    backgroundColor: "rgba(244,63,94,.12)",
  },
  chipText: { color: "white", fontSize: 14, fontWeight: "600", flexShrink: 1 },
  chipIncludeText: { color: "black" },
  chipExcludeText: { color: "#fecdd3", textDecorationLine: "line-through" },
  chipCount: { color: palette.muted, fontSize: 12, fontWeight: "700" },
  chipIncludeCount: { color: "#52525b" },
  iconChip: {
    width: 36,
    height: 36,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  moreChip: {
    minHeight: 36,
    justifyContent: "center",
    paddingHorizontal: 13,
  },
  moreText: { color: palette.secondary, fontWeight: "700" },
  savedRow: { flexDirection: "row", alignItems: "center", minHeight: 52 },
  savedMain: { flex: 1, flexDirection: "row", alignItems: "center", gap: 12 },
  savedName: { color: "white", fontWeight: "700", fontSize: 15 },
  savedMeta: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  savedDelete: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  bookmarkRow: { flexDirection: "row", gap: 8, marginTop: 8 },
  input: {
    flex: 1,
    height: 46,
    borderRadius: 10,
    backgroundColor: "#202024",
    color: "white",
    paddingHorizontal: 13,
  },
  primary: {
    height: 46,
    borderRadius: 23,
    backgroundColor: "white",
    paddingHorizontal: 18,
    justifyContent: "center",
  },
  primaryText: { color: "black", fontWeight: "800" },
  disabled: { opacity: 0.45 },
  smart: {
    marginTop: 22,
    minHeight: 50,
    borderRadius: 25,
    backgroundColor: "white",
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  smartText: { color: "black", fontWeight: "800", fontSize: 15 },
  message: { color: palette.secondary, marginTop: 12, textAlign: "center" },
});
