import { getTags, tagCountsSummary } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Hash, Sparkles } from "lucide-react-native";
import {
  ActivityIndicator,
  FlatList,
  Pressable,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import {
  BrowseHeader,
  browseStyles as styles,
} from "@/components/browse-screen";
import { Screen } from "@/components/music-ui";
import { tagsKey } from "@/components/tag-picker";
import { palette } from "@/constants/colors";
import { tagHex } from "@/lib/browse-collections";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** Every tag, manual and smart. */
export default function TagsScreen() {
  const router = useRouter();
  const tags = useQuery({ queryKey: tagsKey, queryFn: getTags });
  const all = tags.data ?? [];
  const smart = all.filter((tag) => tag.kind === "smart").length;
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <FlatList
          contentContainerStyle={{ paddingBottom: 140 }}
          data={all}
          keyExtractor={(tag) => String(tag.id)}
          ListEmptyComponent={
            tags.isPending ? (
              <ActivityIndicator color="white" style={{ marginTop: 40 }} />
            ) : (
              <View style={styles.empty}>
                <Hash color={palette.accent} size={36} />
                <Text style={[styles.emptyTitle, { marginTop: 14 }]}>
                  No tags yet
                </Text>
                <Text style={styles.emptyText}>
                  Use Tags… in any album, artist or track menu. Smart tags are
                  saved from a Library focus.
                </Text>
              </View>
            )
          }
          ListHeaderComponent={
            <BrowseHeader
              accent={palette.accent}
              kicker="Browse"
              subtitle={
                tags.data
                  ? `${all.length} ${all.length === 1 ? "tag" : "tags"}${smart ? ` · ${smart} smart` : ""}`
                  : undefined
              }
              title="Tags"
            />
          }
          renderItem={({ item: tag }) => {
            const color = tagHex(tag);
            return (
              <Pressable
                {...immediatePressFeedback}
                accessibilityRole="link"
                onPress={() => router.push(`/tag/${tag.id}`)}
                style={styles.row}
              >
                <View
                  style={{
                    width: 52,
                    height: 52,
                    borderRadius: 14,
                    alignItems: "center",
                    justifyContent: "center",
                    backgroundColor: `${color}26`,
                    borderWidth: 1,
                    borderColor: `${color}55`,
                  }}
                >
                  {tag.kind === "smart" ? (
                    <Sparkles color={color} size={22} />
                  ) : (
                    <Hash color={color} size={22} />
                  )}
                </View>
                <View style={styles.rowText}>
                  <Text numberOfLines={1} style={styles.rowTitle}>
                    {tag.name}
                  </Text>
                  <Text numberOfLines={1} style={styles.rowMeta}>
                    {tag.kind === "smart" ? "Smart · " : ""}
                    {tagCountsSummary(tag)}
                  </Text>
                  {tag.description ? (
                    <Text
                      numberOfLines={1}
                      style={[styles.rowMeta, { color: palette.muted }]}
                    >
                      {tag.description}
                    </Text>
                  ) : null}
                </View>
              </Pressable>
            );
          }}
        />
      </SafeAreaView>
    </Screen>
  );
}
