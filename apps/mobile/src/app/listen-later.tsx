import {
  getListenLater,
  removeFromListenLater,
  setListenLaterRemoveWhenPlayed,
  addedAgo,
  type ListenLaterEntity,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Clock, X } from "lucide-react-native";
import { useMemo, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  Pressable,
  ScrollView,
  Switch,
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
import { listenLaterKey } from "@/components/listen-later";
import { Screen } from "@/components/music-ui";
import { palette } from "@/constants/colors";
import { useZonePlayback } from "@/hooks/use-zone-playback";
import { listenLaterRows } from "@/lib/browse-collections";
import { playableCatalogSong } from "@/lib/playable-song";
import { immediatePressFeedback } from "@/lib/press-feedback";

type Filter = "all" | ListenLaterEntity;
const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "album", label: "Albums" },
  { value: "track", label: "Tracks" },
  { value: "artist", label: "Artists" },
];

/** Albums, tracks and artists saved for later. */
export default function ListenLaterScreen() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const playback = useZonePlayback();
  const [filter, setFilter] = useState<Filter>("all");
  const page = useQuery({
    queryKey: [...listenLaterKey, "page", filter],
    queryFn: () => getListenLater({ type: filter, limit: 1000 }),
  });
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: listenLaterKey });
  const remove = useMutation({
    mutationFn: ({ type, id }: { type: ListenLaterEntity; id: string }) =>
      removeFromListenLater(type, [id]),
    onSuccess: refresh,
  });
  const setting = useMutation({
    mutationFn: setListenLaterRemoveWhenPlayed,
    onSuccess: refresh,
  });
  const rows = useMemo(
    () => listenLaterRows(page.data?.items ?? []),
    [page.data],
  );
  const tracks = rows
    .map((row) => row.item.song)
    .filter((song) => song !== undefined)
    .map((song) =>
      playableCatalogSong({ ...song, duration: song.durationSeconds }),
    );
  const counts = page.data?.counts;
  const subtitle = counts
    ? counts.all
      ? [
          counts.album && `${counts.album} albums`,
          counts.track && `${counts.track} tracks`,
          counts.artist && `${counts.artist} artists`,
        ]
          .filter(Boolean)
          .join(" · ")
      : "Music you set aside, in one place."
    : undefined;

  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <FlatList
          contentContainerStyle={{ paddingBottom: 140 }}
          data={rows}
          keyExtractor={(row) => row.key}
          ListEmptyComponent={
            page.isPending ? (
              <ActivityIndicator color="white" style={{ marginTop: 40 }} />
            ) : (
              <View style={styles.empty}>
                <Clock color={palette.accent} size={36} />
                <Text style={[styles.emptyTitle, { marginTop: 14 }]}>
                  Nothing saved for later
                </Text>
                <Text style={styles.emptyText}>
                  Choose Listen later from any album, track or artist menu.
                </Text>
              </View>
            )
          }
          ListHeaderComponent={
            <>
              <BrowseHeader
                accent={palette.accent}
                kicker="Browse"
                subtitle={subtitle}
                title="Listen Later"
              />
              <View
                style={[
                  styles.row,
                  {
                    marginHorizontal: 20,
                    marginBottom: 12,
                    paddingHorizontal: 14,
                    borderRadius: 14,
                    backgroundColor: palette.elevated,
                  },
                ]}
              >
                <View style={styles.rowText}>
                  <Text style={styles.rowTitle}>Remove once played</Text>
                  <Text style={styles.rowMeta}>
                    Tracks after a play, albums after most of them
                  </Text>
                </View>
                <Switch
                  accessibilityLabel="Remove once played"
                  disabled={setting.isPending || page.isPending}
                  onValueChange={(value) => setting.mutate(value)}
                  trackColor={{ true: palette.accent }}
                  value={Boolean(page.data?.removeWhenPlayed)}
                />
              </View>
              <ScrollView
                contentContainerStyle={styles.chips}
                horizontal
                showsHorizontalScrollIndicator={false}
              >
                {FILTERS.map((option) => (
                  <Chip
                    active={filter === option.value}
                    key={option.value}
                    label={option.label}
                    onPress={() => setFilter(option.value)}
                  />
                ))}
              </ScrollView>
            </>
          }
          renderItem={({ item: row }) => (
            <Pressable
              {...immediatePressFeedback}
              accessibilityRole="button"
              onPress={() => {
                const { item } = row;
                if (item.album) router.push(`/album/${item.entityId}`);
                else if (item.artist) router.push(`/artist/${item.entityId}`);
                else {
                  const song = tracks.find(
                    (track) => track.id === item.entityId,
                  );
                  if (song) playback.playSong(song, tracks);
                }
              }}
              style={styles.row}
            >
              <Artwork
                path={row.artwork}
                rounded={row.round ? 28 : 8}
                size={56}
              />
              <View style={styles.rowText}>
                <Text numberOfLines={1} style={styles.rowTitle}>
                  {row.title}
                </Text>
                <Text numberOfLines={1} style={styles.rowMeta}>
                  {row.subtitle}
                </Text>
                <Text
                  style={[
                    styles.rowMeta,
                    { color: palette.muted, fontSize: 12 },
                  ]}
                >
                  {addedAgo(row.item.addedAt)}
                </Text>
              </View>
              <Pressable
                accessibilityLabel={`Remove ${row.title} from Listen Later`}
                accessibilityRole="button"
                hitSlop={10}
                onPress={() =>
                  remove.mutate({
                    type: row.item.entityType,
                    id: row.item.entityId,
                  })
                }
              >
                <X color={palette.secondary} size={20} />
              </Pressable>
            </Pressable>
          )}
        />
      </SafeAreaView>
    </Screen>
  );
}
