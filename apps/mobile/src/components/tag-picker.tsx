import {
  createTag,
  getAssignedTags,
  getTags,
  tagItems,
  untagItems,
  type TaggedEntity,
  type UserTag,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Plus, Tag, X } from "lucide-react-native";
import { useState } from "react";
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
import { SafeAreaView } from "react-native-safe-area-context";

import { DrawerAction } from "@/components/action-drawer";
import { palette } from "@/constants/colors";
import { tagHex } from "@/lib/browse-collections";
import { immediatePressFeedback } from "@/lib/press-feedback";
import { useSession } from "@/providers/session-provider";

export const tagsKey = ["tags"] as const;

/** Tick tags for one album, track or artist; create new ones inline. */
export function TagPicker({
  entityId,
  entityType,
  onClose,
  title,
  visible,
}: {
  entityId: string;
  entityType: TaggedEntity;
  onClose: () => void;
  title?: string;
  visible: boolean;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const tags = useQuery({
    queryKey: tagsKey,
    queryFn: getTags,
    enabled: visible,
  });
  const assigned = useQuery({
    queryKey: [...tagsKey, "assigned", entityType, entityId],
    queryFn: () => getAssignedTags(entityType, entityId),
    enabled: visible,
  });
  const invalidate = () => queryClient.invalidateQueries({ queryKey: tagsKey });
  const toggle = useMutation({
    mutationFn: async ({ tag, add }: { tag: UserTag; add: boolean }) => {
      if (add) await tagItems(tag.id, entityType, [entityId]);
      else await untagItems(tag.id, entityType, [entityId]);
    },
    onSuccess: invalidate,
  });
  const create = useMutation({
    mutationFn: async (value: string) => {
      const tag = await createTag(value);
      await tagItems(tag.id, entityType, [entityId]);
    },
    onSuccess: () => {
      setName("");
      void invalidate();
    },
  });
  const assignedIds = new Set(assigned.data ?? []);
  const manual = (tags.data ?? []).filter((tag) => tag.kind !== "smart");
  return (
    <Modal
      animationType="slide"
      onRequestClose={onClose}
      presentationStyle="pageSheet"
      visible={visible}
    >
      <SafeAreaView edges={["top", "bottom"]} style={styles.sheet}>
        <View style={styles.header}>
          <View style={{ flex: 1 }}>
            <Text style={styles.title}>Tags</Text>
            {title ? (
              <Text numberOfLines={1} style={styles.subtitle}>
                {title}
              </Text>
            ) : null}
          </View>
          <Pressable
            accessibilityLabel="Close"
            accessibilityRole="button"
            hitSlop={12}
            onPress={onClose}
          >
            <X color="white" size={24} />
          </Pressable>
        </View>
        <View style={styles.newRow}>
          <TextInput
            accessibilityLabel="New tag name"
            maxLength={100}
            onChangeText={setName}
            onSubmitEditing={() => name.trim() && create.mutate(name.trim())}
            placeholder="New tag"
            placeholderTextColor={palette.muted}
            returnKeyType="done"
            style={styles.input}
            value={name}
          />
          <Pressable
            {...immediatePressFeedback}
            accessibilityLabel="Create tag"
            accessibilityRole="button"
            disabled={!name.trim() || create.isPending}
            onPress={() => create.mutate(name.trim())}
            style={[styles.add, !name.trim() && { opacity: 0.4 }]}
          >
            {create.isPending ? (
              <ActivityIndicator color="black" />
            ) : (
              <Plus color="black" size={20} />
            )}
          </Pressable>
        </View>
        {tags.isPending || assigned.isPending ? (
          <ActivityIndicator color="white" style={{ marginTop: 40 }} />
        ) : (
          <ScrollView contentContainerStyle={{ paddingBottom: 40 }}>
            {manual.map((tag) => {
              const checked = assignedIds.has(tag.id);
              return (
                <Pressable
                  {...immediatePressFeedback}
                  accessibilityRole="checkbox"
                  accessibilityState={{ checked }}
                  key={tag.id}
                  onPress={() => toggle.mutate({ tag, add: !checked })}
                  style={styles.row}
                >
                  <View style={[styles.box, checked && styles.boxChecked]}>
                    {checked ? (
                      <Check color="black" size={15} strokeWidth={3} />
                    ) : null}
                  </View>
                  <View
                    style={[styles.dot, { backgroundColor: tagHex(tag) }]}
                  />
                  <Text numberOfLines={1} style={styles.rowText}>
                    {tag.name}
                  </Text>
                  <Text style={styles.count}>{tag.item_count}</Text>
                </Pressable>
              );
            })}
            {!manual.length ? (
              <Text style={styles.empty}>
                No tags yet. Name one above to create it.
              </Text>
            ) : null}
          </ScrollView>
        )}
      </SafeAreaView>
    </Modal>
  );
}

/** Drawer entry that opens the tag picker. */
export function TagsAction({
  entityId,
  entityType,
  title,
}: {
  entityId: string;
  entityType: TaggedEntity;
  title?: string;
}) {
  const session = useSession();
  const [open, setOpen] = useState(false);
  if (session.phase !== "ready") return null;
  return (
    <>
      <DrawerAction icon={Tag} label="Tags…" onPress={() => setOpen(true)} />
      <TagPicker
        entityId={entityId}
        entityType={entityType}
        onClose={() => setOpen(false)}
        title={title}
        visible={open}
      />
    </>
  );
}

const styles = StyleSheet.create({
  sheet: { flex: 1, backgroundColor: palette.elevated },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 16,
  },
  title: { color: "white", fontSize: 22, fontWeight: "800" },
  subtitle: { color: palette.secondary, fontSize: 13, marginTop: 2 },
  newRow: {
    flexDirection: "row",
    gap: 10,
    paddingHorizontal: 20,
    marginBottom: 10,
  },
  input: {
    flex: 1,
    height: 44,
    borderRadius: 22,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.borderStrong,
    color: "white",
    paddingHorizontal: 16,
    fontSize: 15,
  },
  add: {
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: "white",
    alignItems: "center",
    justifyContent: "center",
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 13,
  },
  box: {
    width: 22,
    height: 22,
    borderRadius: 6,
    borderWidth: 1.5,
    borderColor: palette.muted,
    alignItems: "center",
    justifyContent: "center",
  },
  boxChecked: { backgroundColor: "white", borderColor: "white" },
  dot: { width: 9, height: 9, borderRadius: 5 },
  rowText: { flex: 1, color: "white", fontSize: 16, fontWeight: "600" },
  count: { color: palette.muted, fontSize: 13, fontVariant: ["tabular-nums"] },
  empty: {
    color: palette.secondary,
    textAlign: "center",
    marginTop: 30,
    paddingHorizontal: 40,
  },
});
