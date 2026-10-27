import { genreChipTitle, genreDisplayName } from "@parson/music-sdk";
import type { GenreRef } from "@parson/music-sdk/types";
import { useRouter } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { genreRoute } from "@/lib/credit-links";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** Several genres as chips that open the genre page. */
export function GenreChips({
  genres,
  centered = false,
}: {
  genres: readonly GenreRef[] | null | undefined;
  centered?: boolean;
}) {
  const router = useRouter();
  if (!genres?.length) return null;
  return (
    <View
      accessibilityLabel="Genres"
      style={[styles.row, centered && styles.centered]}
    >
      {genres.map((genre) => (
        <Pressable
          {...immediatePressFeedback}
          accessibilityHint={genreChipTitle(genre)}
          accessibilityLabel={genre.name}
          accessibilityRole="link"
          key={genre.slug}
          onPress={() => router.push(genreRoute(genre.slug))}
          style={styles.chip}
        >
          <Text style={styles.text}>{genreDisplayName(genre.name)}</Text>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", flexWrap: "wrap", gap: 6, marginTop: 10 },
  centered: { justifyContent: "center" },
  chip: {
    borderColor: "rgba(255,255,255,0.14)",
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    backgroundColor: "rgba(255,255,255,0.06)",
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  text: { color: palette.secondary, fontSize: 12, fontWeight: "600" },
});
