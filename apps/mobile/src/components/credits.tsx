import {
  editorialSourceName,
  groupCredits,
  paragraphs,
  saveNote,
  type AboutCredit,
  type EditorialText,
  type UserNote,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Lock } from "lucide-react-native";
import { useState } from "react";
import {
  Linking,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";

import { PersonAvatar } from "@/components/person-avatar";
import { palette } from "@/constants/colors";
import { artistRoute } from "@/lib/credit-links";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** Wikipedia text with the CC BY-SA attribution it needs. */
export function EditorialBlock({ text }: { text: EditorialText }) {
  const [expanded, setExpanded] = useState(false);
  const blocks = paragraphs(text.body);
  const long = text.body.length > 320 || blocks.length > 1;
  return (
    <View style={styles.block}>
      <Text numberOfLines={expanded ? undefined : 6} style={styles.body}>
        {(expanded ? blocks : blocks.slice(0, 2)).join("\n\n")}
      </Text>
      {long ? (
        <Pressable
          accessibilityRole="button"
          hitSlop={8}
          onPress={() => setExpanded((value) => !value)}
        >
          <Text style={styles.more}>
            {expanded ? "Show less" : "Read more"}
          </Text>
        </Pressable>
      ) : null}
      <Text style={styles.attribution}>
        From the {editorialSourceName(`${text.source}:${text.language}`)}{" "}
        article{" "}
        <Text
          accessibilityRole="link"
          onPress={() => void Linking.openURL(text.url)}
          style={styles.link}
        >
          “{text.title}”
        </Text>
        , licensed under{" "}
        <Text
          accessibilityRole="link"
          onPress={() => void Linking.openURL(text.license_url)}
          style={styles.link}
        >
          {text.license}
        </Text>
        .
      </Text>
    </View>
  );
}

/** Liner-notes credits; each person opens everything they are credited on. */
export function CreditsSection({
  credits,
}: {
  credits: readonly AboutCredit[];
}) {
  const router = useRouter();
  const groups = groupCredits(credits);
  if (!groups.length) return null;
  return (
    <View style={styles.credits}>
      {groups.map((group) => (
        <View key={group.key} style={styles.group}>
          <Text style={styles.role}>{group.label}</Text>
          <View style={styles.people}>
            {group.credits.map((credit) => (
              <Pressable
                {...immediatePressFeedback}
                accessibilityHint="Shows everything this person is credited on"
                accessibilityRole="button"
                disabled={!credit.id}
                key={`${credit.id ?? ""}-${credit.name}`}
                onPress={() =>
                  credit.id && router.push(artistRoute(credit.id, "credits"))
                }
                style={styles.personRow}
              >
                <PersonAvatar
                  imageUrl={credit.image_url}
                  name={credit.name}
                  size={34}
                />
                <Text numberOfLines={1} style={styles.person}>
                  {credit.name}
                  {credit.credited_as?.length ? (
                    <Text style={styles.creditedAs}>
                      {`  as ${credit.credited_as.join(", ")}`}
                    </Text>
                  ) : null}
                </Text>
              </Pressable>
            ))}
          </View>
        </View>
      ))}
    </View>
  );
}

/** The listener's private note about an album or artist. */
export function NoteBlock({
  entityType,
  id,
  note,
}: {
  entityType: "album" | "artist";
  id: string;
  note: UserNote | null | undefined;
}) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(note?.body ?? "");
  const [editing, setEditing] = useState(false);
  const save = useMutation({
    mutationFn: (body: string) => saveNote(entityType, id, body),
    onSuccess: async () => {
      setEditing(false);
      await queryClient.invalidateQueries({
        queryKey: [`${entityType}-about`, id],
      });
    },
  });
  if (!editing) {
    return (
      <Pressable
        accessibilityRole="button"
        onPress={() => {
          setDraft(note?.body ?? "");
          setEditing(true);
        }}
        style={styles.block}
      >
        <Text style={note ? styles.body : styles.placeholder}>
          {note?.body ?? "Add a private note…"}
        </Text>
        <View style={styles.privateRow}>
          <Lock color={palette.muted} size={12} />
          <Text style={styles.attribution}>
            {note ? "Your private note · Tap to edit" : "Only you can see it"}
          </Text>
        </View>
      </Pressable>
    );
  }
  return (
    <View style={styles.block}>
      <TextInput
        accessibilityLabel={`Your note about this ${entityType}`}
        autoFocus
        maxLength={20_000}
        multiline
        onChangeText={setDraft}
        placeholder="Your thoughts…"
        placeholderTextColor={palette.muted}
        style={styles.input}
        value={draft}
      />
      <View style={styles.noteActions}>
        <Pressable
          accessibilityRole="button"
          hitSlop={8}
          onPress={() => {
            setDraft(note?.body ?? "");
            setEditing(false);
          }}
        >
          <Text style={styles.more}>Cancel</Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          disabled={save.isPending}
          hitSlop={8}
          onPress={() => save.mutate(draft)}
        >
          <Text style={styles.save}>{save.isPending ? "Saving…" : "Save"}</Text>
        </Pressable>
      </View>
      {save.isError ? (
        <Text style={styles.attribution}>Could not save your note.</Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  creditedAs: { color: palette.muted, fontSize: 13, fontWeight: "400" },
  block: { paddingHorizontal: 20, gap: 8 },
  body: { color: "#c7c2b9", fontSize: 15, lineHeight: 23 },
  placeholder: { color: palette.muted, fontSize: 15, lineHeight: 23 },
  more: { color: palette.secondary, fontSize: 14, fontWeight: "700" },
  save: { color: "white", fontSize: 14, fontWeight: "800" },
  attribution: { color: palette.muted, fontSize: 12, lineHeight: 17 },
  link: { color: palette.secondary, textDecorationLine: "underline" },
  credits: { paddingHorizontal: 20, gap: 18 },
  group: { gap: 6 },
  role: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1,
    textTransform: "uppercase",
  },
  people: { gap: 8 },
  personRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  person: { color: "white", fontSize: 15, fontWeight: "600", flexShrink: 1 },
  privateRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  input: {
    minHeight: 110,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: palette.border,
    color: "white",
    fontSize: 15,
    padding: 12,
    textAlignVertical: "top",
  },
  noteActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: 22,
  },
});
