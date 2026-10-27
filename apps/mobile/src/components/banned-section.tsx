import {
  clearRadioDislike,
  filterRadioBans,
  getRadioDislikes,
  radioBanKindLabel,
  type RadioBan,
  type RadioBanFilter,
  type RadioDislike,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Ban } from "lucide-react-native";
import { useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { Artwork } from "@/components/artwork";
import { useRadioBans } from "@/components/radio-bans";
import { palette } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";

type Filter = RadioBanFilter | "disliked";

const filters: { id: Filter; label: string }[] = [
  { id: "all", label: "All bans" },
  { id: "artist", label: "Artists" },
  { id: "album", label: "Albums" },
  { id: "song", label: "Tracks" },
  { id: "disliked", label: "Thumbs down" },
];

const dislikesKey = ["radio-dislikes"] as const;

/**
 * Library > Banned: what Radio, Shuffle and recommendations skip, with
 * one-tap unban, plus thumbs-down tracks that can each be undone.
 */
export function BannedSection() {
  const [filter, setFilter] = useState<Filter>("all");
  const bans = useRadioBans();
  const client = useQueryClient();
  const router = useRouter();
  const dislikes = useQuery({
    queryKey: dislikesKey,
    queryFn: getRadioDislikes,
    enabled: bans.online,
    staleTime: 30_000,
  });
  const undo = useMutation({
    mutationFn: (songId: string) => clearRadioDislike(songId),
    onMutate: async (songId) => {
      await client.cancelQueries({ queryKey: dislikesKey });
      const previous = client.getQueryData<RadioDislike[]>(dislikesKey);
      client.setQueryData<RadioDislike[]>(dislikesKey, (current) =>
        (current ?? []).filter((item) => item.song.id !== songId),
      );
      return { previous };
    },
    onError: (_error, _songId, context) =>
      client.setQueryData(dislikesKey, context?.previous),
    onSettled: () => void client.invalidateQueries({ queryKey: dislikesKey }),
  });
  const [failed, setFailed] = useState<string | null>(null);

  const count = (id: Filter) =>
    id === "disliked"
      ? (dislikes.data?.length ?? 0)
      : filterRadioBans(bans.bans, id).length;
  const items = filter === "disliked" ? [] : filterRadioBans(bans.bans, filter);
  const loading = filter === "disliked" ? dislikes.isPending : bans.loading;

  const open = (ban: RadioBan) => {
    if (!ban.name) return;
    if (ban.kind === "artist") router.push(`/artist/${ban.id}`);
    else if (ban.kind === "album") router.push(`/album/${ban.id}`);
    else if (ban.album_id) router.push(`/album/${ban.album_id}`);
  };

  return (
    <View>
      <Text style={styles.intro}>
        Banned artists, albums and tracks never play in Radio, Shuffle or
        recommendations, from the next pick on. You can still play them
        yourself.
      </Text>
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={styles.filters}
      >
        {filters.map((item) => (
          <Pressable
            accessibilityRole="radio"
            accessibilityState={{ selected: filter === item.id }}
            key={item.id}
            onPress={() => setFilter(item.id)}
            style={[styles.filter, filter === item.id && styles.filterActive]}
          >
            <Text
              style={[
                styles.filterText,
                filter === item.id && styles.filterTextActive,
              ]}
            >
              {item.label} {count(item.id)}
            </Text>
          </Pressable>
        ))}
      </ScrollView>
      {failed ? (
        <Text accessibilityRole="alert" style={styles.error}>
          {failed}
        </Text>
      ) : null}
      {loading ? (
        <ActivityIndicator color={palette.text} style={{ marginTop: 40 }} />
      ) : filter === "disliked" ? (
        dislikes.data?.length ? (
          dislikes.data.map(({ song }) => (
            <View key={song.id} style={styles.row}>
              <Artwork path={song.album_object?.cover_url} size={52} />
              <View style={styles.labels}>
                <Text numberOfLines={1} style={styles.name}>
                  {song.name}
                </Text>
                <Text numberOfLines={1} style={styles.meta}>
                  {song.artist} • {song.album_object?.name}
                </Text>
              </View>
              <Pressable
                {...immediatePressFeedback}
                accessibilityLabel={`Undo thumbs down on ${song.name}`}
                accessibilityRole="button"
                style={styles.action}
                onPress={() => {
                  setFailed(null);
                  undo.mutate(song.id, {
                    onError: () =>
                      setFailed("Could not undo that thumbs down."),
                  });
                }}
              >
                <Text style={styles.actionText}>Undo</Text>
              </Pressable>
            </View>
          ))
        ) : (
          <Empty
            title="No thumbs down"
            body="Tracks you thumb down in Radio show up here so you can change your mind."
          />
        )
      ) : items.length ? (
        items.map((ban) => (
          <Pressable
            accessibilityRole="button"
            key={`${ban.kind}:${ban.id}`}
            onPress={() => open(ban)}
            style={styles.row}
          >
            <View>
              <Artwork
                path={ban.image_url}
                rounded={ban.kind === "artist" ? 26 : 8}
                size={52}
              />
              <View style={styles.badge}>
                <Ban color={palette.danger} size={13} />
              </View>
            </View>
            <View style={styles.labels}>
              <Text numberOfLines={1} style={styles.name}>
                {ban.name ?? "No longer in your library"}
              </Text>
              <Text numberOfLines={1} style={styles.meta}>
                {radioBanKindLabel(ban.kind).replace(/^./, (c) =>
                  c.toUpperCase(),
                )}
                {ban.subtitle && ban.kind !== "artist"
                  ? ` • ${ban.subtitle}`
                  : ""}
              </Text>
            </View>
            <Pressable
              {...immediatePressFeedback}
              accessibilityLabel={`Unban ${ban.name ?? radioBanKindLabel(ban.kind)}`}
              accessibilityRole="button"
              style={styles.action}
              onPress={() => {
                setFailed(null);
                void bans
                  .setBanned({ kind: ban.kind, id: ban.id }, false)
                  .then((saved) => {
                    if (!saved) setFailed("Could not unban that.");
                  });
              }}
            >
              <Text style={styles.actionText}>Unban</Text>
            </Pressable>
          </Pressable>
        ))
      ) : (
        <Empty
          title="Nothing banned here"
          body="Open the … menu on an artist, album or song and choose Ban to keep it out of Radio and Shuffle."
        />
      )}
    </View>
  );
}

function Empty({ body, title }: { body: string; title: string }) {
  return (
    <View style={styles.empty}>
      <Text style={styles.emptyTitle}>{title}</Text>
      <Text style={styles.emptyText}>{body}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  intro: {
    color: palette.secondary,
    fontSize: 13,
    lineHeight: 19,
    paddingHorizontal: 20,
    paddingBottom: 12,
  },
  filters: { flexDirection: "row", gap: 8, paddingHorizontal: 20 },
  filter: {
    borderColor: palette.border,
    borderRadius: 15,
    borderWidth: 1,
    height: 30,
    justifyContent: "center",
    paddingHorizontal: 12,
  },
  filterActive: {
    backgroundColor: palette.elevatedStrong,
    borderColor: palette.text,
  },
  filterText: { color: palette.secondary, fontSize: 12, fontWeight: "700" },
  filterTextActive: { color: palette.text },
  error: { color: palette.danger, paddingHorizontal: 20, paddingTop: 10 },
  row: {
    alignItems: "center",
    flexDirection: "row",
    gap: 13,
    minHeight: 72,
    paddingHorizontal: 20,
  },
  badge: {
    backgroundColor: "rgba(0,0,0,0.8)",
    borderRadius: 9,
    bottom: -2,
    padding: 2,
    position: "absolute",
    right: -2,
  },
  labels: { flex: 1 },
  name: { color: palette.text, fontSize: 15, fontWeight: "700" },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 4 },
  action: {
    borderColor: palette.borderStrong,
    borderRadius: 16,
    borderWidth: 1,
    height: 32,
    justifyContent: "center",
    paddingHorizontal: 14,
  },
  actionText: { color: palette.text, fontSize: 13, fontWeight: "700" },
  empty: { alignItems: "center", paddingHorizontal: 32, paddingTop: 48 },
  emptyTitle: { color: palette.text, fontSize: 17, fontWeight: "700" },
  emptyText: {
    color: palette.secondary,
    fontSize: 14,
    lineHeight: 20,
    marginTop: 8,
    textAlign: "center",
  },
});
