import {
  editorialSourceName,
  editorialSourceShortName,
  reviewStars,
  type ReviewScores,
  type TextSource,
} from "@parson/music-sdk";
import { Star } from "lucide-react-native";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";

import { palette, tint } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** One chip per editorial source with text; the active one filled. */
export function TextSourceChips({
  sources,
  active,
  onPick,
}: {
  sources: readonly TextSource[];
  active: string | null | undefined;
  onPick: (id: string) => void;
}) {
  if (sources.length < 2) return null;
  return (
    <View accessibilityRole="radiogroup" style={styles.row}>
      <Text style={styles.label}>Source</Text>
      {sources.map((source) => {
        const selected = source.id === active;
        return (
          <Pressable
            {...immediatePressFeedback}
            accessibilityLabel={editorialSourceName(source.id)}
            accessibilityRole="radio"
            accessibilityState={{ checked: selected }}
            key={source.id}
            onPress={() => onPick(source.id)}
            style={[styles.chip, selected && styles.chipActive]}
          >
            <Text style={[styles.chipText, selected && styles.chipTextActive]}>
              {editorialSourceShortName(source.id)}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

/** Publications' ratings from the article's review scores table. */
export function ReviewScoresBlock({ scores }: { scores: ReviewScores }) {
  return (
    <View style={styles.scores}>
      <Text accessibilityRole="header" style={styles.scoresTitle}>
        Professional ratings
      </Text>
      {scores.scores.map((score) => {
        const stars = reviewStars(score);
        return (
          <View key={score.publication} style={styles.scoreRow}>
            <Text numberOfLines={1} style={styles.publication}>
              {score.publication}
            </Text>
            {stars ? (
              <View
                accessibilityLabel={`${score.rating}, ${stars.value} of 5 stars`}
                accessible
                style={styles.stars}
              >
                {Array.from({ length: 5 }, (_, index) => (
                  <Star
                    color={
                      index < stars.full + stars.half
                        ? palette.brass
                        : tint(0.18)
                    }
                    fill={index < stars.full ? palette.brass : "transparent"}
                    key={index}
                    size={13}
                  />
                ))}
              </View>
            ) : (
              <Text style={styles.rating}>{score.rating}</Text>
            )}
          </View>
        );
      })}
      <Text style={styles.attribution}>
        As listed in the {editorialSourceName(`wikipedia:${scores.language}`)}{" "}
        article{" "}
        <Text
          accessibilityRole="link"
          onPress={() => void Linking.openURL(scores.url)}
          style={styles.link}
        >
          “{scores.title}”
        </Text>
        , licensed under{" "}
        <Text
          accessibilityRole="link"
          onPress={() => void Linking.openURL(scores.license_url)}
          style={styles.link}
        >
          {scores.license}
        </Text>
        .
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 6,
    marginBottom: 10,
  },
  label: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 0.8,
    textTransform: "uppercase",
    marginRight: 2,
  },
  chip: {
    borderRadius: 999,
    borderWidth: 1,
    borderColor: tint(0.12),
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  chipActive: {
    borderColor: tint(0.45),
    backgroundColor: tint(0.12),
  },
  chipText: { color: palette.secondary, fontSize: 12, fontWeight: "600" },
  chipTextActive: { color: palette.text },
  scores: {
    marginTop: 14,
    padding: 14,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: tint(0.08),
    backgroundColor: tint(0.03),
    gap: 8,
  },
  scoresTitle: {
    color: palette.secondary,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  scoreRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  publication: { color: palette.secondary, fontSize: 13, flex: 1 },
  stars: { flexDirection: "row", gap: 2 },
  rating: { color: palette.text, fontSize: 13, fontWeight: "700" },
  attribution: { color: palette.muted, fontSize: 11, lineHeight: 16 },
  link: { color: palette.secondary, textDecorationLine: "underline" },
});
