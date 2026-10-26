import {
  getImportJob,
  importErrorMessage,
  saveImportPlaylist,
  updateImportItems,
  type ImportItem,
  type ImportItemAction,
  type ImportJob,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import {
  ArrowLeft,
  Check,
  ListMusic,
  RefreshCw,
  SkipForward,
} from "lucide-react-native";
import { useMemo, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { ActionDrawer, DrawerAction } from "@/components/action-drawer";
import { Screen } from "@/components/music-ui";
import { layout, palette } from "@/constants/colors";
import {
  acceptSuggestionActions,
  confidencePercent,
  importStatusLine,
  initialMobileFilter,
  isJobRunning,
  itemsForFilter,
  SERVICE_COLORS,
  SERVICE_NAMES,
  type MobileReviewFilter,
} from "@/lib/imports";
import { immediatePressFeedback } from "@/lib/press-feedback";

const FILTERS: { id: MobileReviewFilter; label: string }[] = [
  { id: "review", label: "Check" },
  { id: "unmatched", label: "Not found" },
  { id: "all", label: "All" },
];

function ItemRow({
  item,
  job,
  onChange,
}: {
  item: ImportItem;
  job: ImportJob;
  onChange: (item: ImportItem) => void;
}) {
  const song = item.song_id ? job.songs[item.song_id] : undefined;
  const tone =
    item.status === "matched" || item.status === "confirmed"
      ? "#6ee7b7"
      : item.status === "review"
        ? "#fcd34d"
        : item.status === "unmatched"
          ? "#fda4af"
          : palette.muted;
  return (
    <Pressable
      {...immediatePressFeedback}
      accessibilityHint="Change how this track is imported"
      accessibilityRole="button"
      onPress={() => onChange(item)}
      style={[styles.row, item.status === "skipped" && { opacity: 0.5 }]}
    >
      <View style={{ flex: 1, gap: 2 }}>
        <Text numberOfLines={1} style={styles.title}>
          {item.source.title}
        </Text>
        <Text numberOfLines={1} style={styles.meta}>
          {item.source.artists.join(", ") || "Title only"}
        </Text>
        <Text numberOfLines={1} style={[styles.match, { color: tone }]}>
          {song
            ? `→ ${song.name} · ${song.artist} · ${confidencePercent(item.confidence)}`
            : item.status === "skipped"
              ? "Left out"
              : "Not in your library"}
        </Text>
      </View>
    </Pressable>
  );
}

export default function ImportReviewScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const client = useQueryClient();
  const [filter, setFilter] = useState<MobileReviewFilter | null>(null);
  const [selected, setSelected] = useState<ImportItem | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const query = useQuery({
    queryKey: ["import-job", id],
    queryFn: () => getImportJob(id!),
    enabled: Boolean(id),
    refetchInterval: (state) =>
      state.state.data && isJobRunning(state.state.data) ? 1_000 : false,
  });
  const job = query.data;
  const activeFilter = filter ?? (job ? initialMobileFilter(job.items) : "all");
  const visible = useMemo(
    () => (job ? itemsForFilter(job.items, activeFilter) : []),
    [activeFilter, job],
  );

  const apply = async (actions: ImportItemAction[]) => {
    if (!job || busy || !actions.length) return;
    setBusy(true);
    setMessage("");
    try {
      const next = await updateImportItems(job.id, actions);
      client.setQueryData(["import-job", id], next);
      void client.invalidateQueries({ queryKey: ["import-jobs"] });
    } catch (error) {
      setMessage(importErrorMessage(error, "The change was not saved."));
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    if (!job || busy) return;
    setBusy(true);
    setMessage("");
    try {
      const result = await saveImportPlaylist(job.id);
      client.setQueryData(["import-job", id], {
        ...job,
        playlist_id: result.playlist_id,
      });
      void client.invalidateQueries({ queryKey: ["playlists"] });
      router.push(`/playlist/${result.playlist_id}`);
    } catch (error) {
      setMessage(importErrorMessage(error, "The playlist was not saved."));
    } finally {
      setBusy(false);
    }
  };

  const header = (
    <Pressable
      accessibilityLabel="Back"
      accessibilityRole="button"
      onPress={() => router.back()}
      style={styles.back}
    >
      <ArrowLeft color="white" size={22} />
    </Pressable>
  );

  if (!job) {
    return (
      <Screen>
        <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
          {header}
          {query.isPending ? (
            <ActivityIndicator color="white" style={{ flex: 1 }} />
          ) : (
            <Text style={styles.empty}>This import could not be found.</Text>
          )}
        </SafeAreaView>
      </Screen>
    );
  }

  const suggestions = acceptSuggestionActions(job.items);
  const alternatives =
    selected?.candidates.filter(
      (candidate) =>
        candidate.song_id !== selected.song_id && job.songs[candidate.song_id],
    ) ?? [];

  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        {header}
        <View style={styles.headerBlock}>
          <View style={styles.serviceLine}>
            <View
              style={[
                styles.dot,
                { backgroundColor: SERVICE_COLORS[job.service] },
              ]}
            />
            <Text style={styles.service}>{SERVICE_NAMES[job.service]}</Text>
          </View>
          <Text
            accessibilityRole="header"
            numberOfLines={2}
            style={styles.name}
          >
            {job.name}
          </Text>
          <Text style={styles.meta}>{importStatusLine(job)}</Text>
        </View>
        {isJobRunning(job) || job.status === "failed" ? (
          <View style={styles.pending}>
            {job.status === "failed" ? null : (
              <ActivityIndicator color="white" />
            )}
            <Text style={styles.meta}>{importStatusLine(job)}</Text>
          </View>
        ) : (
          <>
            <View style={styles.filters}>
              {FILTERS.map((option) => (
                <Pressable
                  accessibilityRole="tab"
                  accessibilityState={{ selected: activeFilter === option.id }}
                  key={option.id}
                  onPress={() => setFilter(option.id)}
                  style={[
                    styles.filter,
                    activeFilter === option.id && styles.filterActive,
                  ]}
                >
                  <Text
                    style={[
                      styles.filterText,
                      activeFilter === option.id && styles.filterTextActive,
                    ]}
                  >
                    {option.label} ·{" "}
                    {itemsForFilter(job.items, option.id).length}
                  </Text>
                </Pressable>
              ))}
            </View>
            {suggestions.length ? (
              <Pressable
                {...immediatePressFeedback}
                accessibilityRole="button"
                disabled={busy}
                onPress={() => void apply(suggestions)}
                style={styles.secondary}
              >
                <Check color="white" size={16} />
                <Text style={styles.secondaryText}>
                  Accept {suggestions.length} suggestions
                </Text>
              </Pressable>
            ) : null}
            <FlatList
              contentContainerStyle={{ paddingBottom: 140 }}
              data={visible}
              keyExtractor={(item) => String(item.position)}
              ListEmptyComponent={
                <Text style={styles.empty}>Nothing left here.</Text>
              }
              renderItem={({ item }) => (
                <ItemRow item={item} job={job} onChange={setSelected} />
              )}
            />
            <View style={styles.footer}>
              {message ? <Text style={styles.error}>{message}</Text> : null}
              <Pressable
                {...immediatePressFeedback}
                accessibilityRole="button"
                disabled={busy || job.matched === 0}
                onPress={() => void save()}
                style={[
                  styles.primary,
                  (busy || job.matched === 0) && { opacity: 0.45 },
                ]}
              >
                {busy ? (
                  <ActivityIndicator color="black" />
                ) : (
                  <Text style={styles.primaryText}>
                    {job.playlist_id ? "Update playlist" : "Save as playlist"} (
                    {job.matched})
                  </Text>
                )}
              </Pressable>
            </View>
          </>
        )}
        <ActionDrawer
          onClose={() => setSelected(null)}
          open={selected !== null}
          title={selected?.source.title}
        >
          {selected?.status === "review" && selected.song_id ? (
            <DrawerAction
              icon={Check}
              label="Accept suggested match"
              onPress={() => {
                void apply([{ action: "accept", position: selected.position }]);
                setSelected(null);
              }}
            />
          ) : null}
          {alternatives.map((candidate) => {
            const option = job.songs[candidate.song_id]!;
            return (
              <DrawerAction
                icon={ListMusic}
                key={candidate.song_id}
                label={`${option.name} · ${option.artist} (${confidencePercent(candidate.confidence)})`}
                onPress={() => {
                  void apply([
                    {
                      action: "choose",
                      position: selected!.position,
                      song_id: candidate.song_id,
                    },
                  ]);
                  setSelected(null);
                }}
              />
            );
          })}
          {selected?.status === "skipped" ? (
            <DrawerAction
              icon={RefreshCw}
              label="Include again"
              onPress={() => {
                void apply([{ action: "reset", position: selected.position }]);
                setSelected(null);
              }}
            />
          ) : (
            <DrawerAction
              icon={SkipForward}
              label="Leave out of the playlist"
              onPress={() => {
                if (selected) {
                  void apply([{ action: "skip", position: selected.position }]);
                }
                setSelected(null);
              }}
            />
          )}
        </ActionDrawer>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  back: {
    marginLeft: layout.horizontal - 6,
    marginTop: 8,
    height: 40,
    width: 40,
    alignItems: "center",
    justifyContent: "center",
  },
  headerBlock: {
    paddingHorizontal: layout.horizontal,
    gap: 4,
    marginBottom: 14,
  },
  serviceLine: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { height: 8, width: 8, borderRadius: 4 },
  service: { color: palette.secondary, fontSize: 12, fontWeight: "700" },
  name: { color: "white", fontSize: 26, fontWeight: "800" },
  meta: { color: palette.secondary, fontSize: 13 },
  pending: { alignItems: "center", gap: 12, paddingTop: 60 },
  filters: {
    flexDirection: "row",
    gap: 8,
    paddingHorizontal: layout.horizontal,
    marginBottom: 10,
  },
  filter: {
    borderRadius: 999,
    paddingHorizontal: 12,
    paddingVertical: 6,
    backgroundColor: palette.elevatedStrong,
  },
  filterActive: { backgroundColor: "white" },
  filterText: { color: palette.secondary, fontSize: 13, fontWeight: "600" },
  filterTextActive: { color: "black" },
  secondary: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    alignSelf: "flex-start",
    marginHorizontal: layout.horizontal,
    marginBottom: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: palette.borderStrong,
  },
  secondaryText: { color: "white", fontSize: 13, fontWeight: "600" },
  row: {
    paddingHorizontal: layout.horizontal,
    paddingVertical: 10,
    borderBottomColor: palette.border,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  title: { color: "white", fontSize: 15, fontWeight: "600" },
  match: { fontSize: 12, marginTop: 2 },
  empty: {
    color: palette.secondary,
    textAlign: "center",
    paddingVertical: 40,
  },
  footer: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: layout.miniPlayer + 12,
    paddingHorizontal: layout.horizontal,
    gap: 8,
  },
  primary: {
    backgroundColor: "white",
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
    height: 48,
  },
  primaryText: { color: "black", fontWeight: "800" },
  error: { color: palette.danger, fontSize: 13, textAlign: "center" },
});
