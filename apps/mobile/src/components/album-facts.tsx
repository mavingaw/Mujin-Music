import {
  ALBUM_FIELD_LABELS,
  factSourceLabel,
  formatPartialDate,
  isTextField,
} from "@parson/music-sdk";
import type { AlbumSources, ResolvedFact } from "@parson/music-sdk";
import { StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";

const SOURCE_COLORS: Record<NonNullable<ResolvedFact["source"]>, string> = {
  edited: "#7dd3fc",
  tags: palette.secondary,
  online: "#c4b5fd",
};

function display(fact: ResolvedFact) {
  if (!fact.value) return null;
  if (fact.field === "original_date" || fact.field === "release_date")
    return formatPartialDate(fact.value) ?? fact.value;
  return fact.value;
}

/**
 * The album's release facts with where each comes from: file tags,
 * MusicBrainz, or a value an administrator froze.
 */
export function AlbumFacts({ sources }: { sources: AlbumSources }) {
  const original = sources.facts.find((fact) => fact.field === "original_date");
  const rows = sources.facts.filter(
    (fact) =>
      isTextField(fact.field) &&
      fact.value &&
      // The edition date only when it differs from the original.
      !(fact.field === "release_date" && fact.value === original?.value),
  );
  if (!rows.length) return null;
  return (
    <View accessibilityLabel="Release facts" style={styles.list}>
      {rows.map((fact) => {
        const source = factSourceLabel(fact);
        return (
          <View key={fact.field} style={styles.row}>
            <Text style={styles.label}>
              {fact.field === "release_date" && !original?.value
                ? "Released"
                : ALBUM_FIELD_LABELS[fact.field]}
            </Text>
            <View style={styles.valueColumn}>
              <Text numberOfLines={1} style={styles.value}>
                {display(fact)}
              </Text>
              {source && fact.source ? (
                <Text
                  style={[styles.source, { color: SOURCE_COLORS[fact.source] }]}
                >
                  {fact.source === "edited" ? "Edited · frozen" : source}
                </Text>
              ) : null}
            </View>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.border,
    backgroundColor: palette.elevated,
    paddingHorizontal: 14,
    paddingVertical: 6,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingVertical: 8,
  },
  label: { color: palette.muted, fontSize: 13, fontWeight: "600" },
  valueColumn: { flexShrink: 1, alignItems: "flex-end" },
  value: { color: palette.text, fontSize: 14, fontWeight: "500" },
  source: {
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 0.6,
    marginTop: 1,
    textTransform: "uppercase",
  },
});
