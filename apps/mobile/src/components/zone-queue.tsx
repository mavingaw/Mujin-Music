import {
  getSongInfos,
  isEditableQueueItem,
  nextRepeatMode,
  previewQueueEdit,
  repeatModeLabel,
  type Zone,
  type ZoneCommand,
} from "@parson/music-sdk";
import type { LibrarySong } from "@parson/music-sdk/types";
import { useQuery } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronUp,
  ListX,
  Repeat,
  Repeat1,
  Shuffle,
  X,
} from "lucide-react-native";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** Most rows shown; a Sonos room maps at most 100 tracks at a time anyway. */
const VISIBLE_ITEMS = 100;

/**
 * Queue management for a remote zone (another app, a Sonos room, a linked
 * group): reorder, remove, clear upcoming, repeat and shuffle, all through
 * the zone command API.
 */
export function ZoneQueue({
  zone,
  send,
}: {
  zone: Zone;
  send: (command: ZoneCommand) => Promise<void>;
}) {
  const [optimistic, setOptimistic] = useState<{
    base: string;
    ids: string[];
  } | null>(null);
  const base = zone.queue.song_ids.join(",");
  const ids =
    optimistic && optimistic.base === base
      ? optimistic.ids
      : zone.queue.song_ids;
  const current = zone.queue.current_position;
  const infos = useQuery({
    queryKey: [
      "zone-queue",
      [...new Set(zone.queue.song_ids)].sort().join(","),
    ],
    queryFn: () => getSongInfos(zone.queue.song_ids, false),
    enabled: zone.queue.song_ids.length > 0,
    staleTime: 5 * 60_000,
  }).data;
  const edit = (command: ZoneCommand) => {
    const preview = previewQueueEdit(ids, current, command);
    if (!preview) return;
    setOptimistic({ base, ids: preview });
    void send(command).catch(() => setOptimistic(null));
  };
  const editable = zone.capabilities.queue_edit && zone.source === "parson";
  const upcoming = Math.max(0, ids.length - current - 1);
  const RepeatIcon = zone.repeat === "one" ? Repeat1 : Repeat;
  const rows = ids
    .map((id, index) => ({ id, index }))
    .slice(current, current + VISIBLE_ITEMS);

  return (
    <View style={styles.queue}>
      <View style={styles.header}>
        <Text style={styles.heading}>
          Queue{upcoming ? ` · ${upcoming} up next` : ""}
        </Text>
        <View style={styles.actions}>
          {zone.capabilities.shuffle ? (
            <Pressable
              {...immediatePressFeedback}
              accessibilityLabel={zone.shuffle ? "Shuffle on" : "Shuffle off"}
              accessibilityRole="button"
              accessibilityState={{ selected: zone.shuffle }}
              hitSlop={6}
              onPress={() =>
                void send({
                  command: "set_shuffle",
                  enabled: !zone.shuffle,
                }).catch(() => {})
              }
              style={[styles.toggle, zone.shuffle && styles.toggleOn]}
            >
              <Shuffle
                color={zone.shuffle ? palette.background : palette.text}
                size={16}
              />
            </Pressable>
          ) : null}
          {zone.capabilities.repeat ? (
            <Pressable
              {...immediatePressFeedback}
              accessibilityLabel={repeatModeLabel(zone.repeat)}
              accessibilityRole="button"
              accessibilityState={{ selected: zone.repeat !== "off" }}
              hitSlop={6}
              onPress={() =>
                void send({
                  command: "set_repeat",
                  mode: nextRepeatMode(zone.repeat),
                }).catch(() => {})
              }
              style={[styles.toggle, zone.repeat !== "off" && styles.toggleOn]}
            >
              <RepeatIcon
                color={
                  zone.repeat !== "off" ? palette.background : palette.text
                }
                size={16}
              />
            </Pressable>
          ) : null}
          {editable ? (
            <Pressable
              {...immediatePressFeedback}
              accessibilityLabel="Clear upcoming"
              accessibilityRole="button"
              accessibilityState={{ disabled: upcoming === 0 }}
              disabled={upcoming === 0}
              hitSlop={6}
              onPress={() => edit({ command: "clear_upcoming" })}
              style={[styles.clear, upcoming === 0 && styles.disabled]}
            >
              <ListX color={palette.text} size={16} />
              <Text style={styles.clearText}>Clear</Text>
            </Pressable>
          ) : null}
        </View>
      </View>
      <ScrollView nestedScrollEnabled style={styles.list}>
        {rows.map(({ id, index }) => {
          const song = infos?.[id] as LibrarySong | undefined;
          const canEdit =
            editable &&
            isEditableQueueItem(
              { ...zone, queue: { song_ids: ids, current_position: current } },
              index,
            );
          const title = song?.name || "Unavailable track";
          return (
            <View key={`${id}-${index}`} style={styles.row}>
              <View style={styles.rowText}>
                <Text
                  numberOfLines={1}
                  style={[styles.title, index === current && styles.playing]}
                >
                  {index === current ? "Now playing · " : ""}
                  {title}
                </Text>
                <Text numberOfLines={1} style={styles.artist}>
                  {song?.artist ?? ""}
                </Text>
              </View>
              {canEdit ? (
                <View style={styles.rowActions}>
                  <Pressable
                    accessibilityLabel={`Move ${title} up`}
                    accessibilityRole="button"
                    accessibilityState={{ disabled: index <= current + 1 }}
                    disabled={index <= current + 1}
                    hitSlop={6}
                    onPress={() =>
                      edit({
                        command: "move_queue_item",
                        from: index,
                        to: index - 1,
                        song_id: id,
                      })
                    }
                    style={[
                      styles.small,
                      index <= current + 1 && styles.disabled,
                    ]}
                  >
                    <ChevronUp color={palette.text} size={16} />
                  </Pressable>
                  <Pressable
                    accessibilityLabel={`Move ${title} down`}
                    accessibilityRole="button"
                    accessibilityState={{ disabled: index >= ids.length - 1 }}
                    disabled={index >= ids.length - 1}
                    hitSlop={6}
                    onPress={() =>
                      edit({
                        command: "move_queue_item",
                        from: index,
                        to: index + 1,
                        song_id: id,
                      })
                    }
                    style={[
                      styles.small,
                      index >= ids.length - 1 && styles.disabled,
                    ]}
                  >
                    <ChevronDown color={palette.text} size={16} />
                  </Pressable>
                  <Pressable
                    accessibilityLabel={`Remove ${title} from the queue`}
                    accessibilityRole="button"
                    hitSlop={6}
                    onPress={() =>
                      edit({
                        command: "remove_queue_item",
                        queue_position: index,
                        song_id: id,
                      })
                    }
                    style={styles.small}
                  >
                    <X color={palette.text} size={16} />
                  </Pressable>
                </View>
              ) : null}
            </View>
          );
        })}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  queue: {
    borderColor: palette.border,
    borderTopWidth: 1,
    marginTop: 14,
    paddingTop: 12,
  },
  header: {
    alignItems: "center",
    flexDirection: "row",
    justifyContent: "space-between",
  },
  heading: { color: palette.text, fontSize: 14, fontWeight: "700" },
  actions: { alignItems: "center", flexDirection: "row", gap: 8 },
  toggle: {
    alignItems: "center",
    borderColor: palette.borderStrong,
    borderRadius: 999,
    borderWidth: 1,
    height: 32,
    justifyContent: "center",
    width: 32,
  },
  toggleOn: { backgroundColor: palette.text, borderColor: palette.text },
  clear: {
    alignItems: "center",
    borderColor: palette.borderStrong,
    borderRadius: 999,
    borderWidth: 1,
    flexDirection: "row",
    gap: 4,
    height: 32,
    paddingHorizontal: 10,
  },
  clearText: { color: palette.text, fontSize: 12, fontWeight: "600" },
  list: { marginTop: 8, maxHeight: 260 },
  row: {
    alignItems: "center",
    flexDirection: "row",
    gap: 8,
    paddingVertical: 7,
  },
  rowText: { flex: 1, minWidth: 0 },
  title: { color: palette.text, fontSize: 14, fontWeight: "500" },
  playing: { color: palette.success },
  artist: { color: palette.secondary, fontSize: 12, marginTop: 1 },
  rowActions: { flexDirection: "row", gap: 6 },
  small: {
    alignItems: "center",
    borderColor: palette.border,
    borderRadius: 999,
    borderWidth: 1,
    height: 30,
    justifyContent: "center",
    width: 30,
  },
  disabled: { opacity: 0.35 },
});
