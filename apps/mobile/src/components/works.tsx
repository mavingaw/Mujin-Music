import {
  composerWorkFacets,
  filterComposerWorks,
  groupWorksByForm,
  workFacts,
  type ComposerWork,
  type FacetValue,
  type WorkFilter,
  type WorkSummary,
} from "@parson/music-sdk";
import { useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { joinMeta, workRoute } from "@/lib/credit-links";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** A piece's heading above its movements in an album's track list. */
export function WorkHeading({ work }: { work: WorkSummary }) {
  const router = useRouter();
  const facts = workFacts(work);
  return (
    <Pressable
      {...immediatePressFeedback}
      accessibilityHint="Opens the work and every recording of it"
      accessibilityLabel={`Go to work ${work.title}`}
      accessibilityRole="link"
      onPress={() => router.push(workRoute(work.mbid))}
      style={styles.heading}
    >
      <Text numberOfLines={2} style={styles.headingTitle}>
        {work.composer ? `${work.composer.name}: ` : ""}
        {work.title}
      </Text>
      {facts.length ? (
        <Text numberOfLines={1} style={styles.headingFacts}>
          {facts.join(" · ")}
        </Text>
      ) : null}
    </Pressable>
  );
}

export function Chip({
  label,
  count,
  active,
  onPress,
}: {
  label: string;
  count?: number;
  active: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ selected: active }}
      onPress={onPress}
      style={[styles.chip, active && styles.chipActive]}
    >
      <Text style={[styles.chipText, active && styles.chipTextActive]}>
        {label}
        {count !== undefined ? `  ${count}` : ""}
      </Text>
    </Pressable>
  );
}

function ChipRail({
  label,
  values,
  selected,
  onSelect,
}: {
  label: string;
  values: FacetValue[];
  selected: string | null | undefined;
  onSelect: (value: string | null) => void;
}) {
  if (!values.length) return null;
  return (
    <ScrollView
      contentContainerStyle={styles.rail}
      horizontal
      showsHorizontalScrollIndicator={false}
    >
      <Text style={styles.railLabel}>{label}</Text>
      {values.map((facet) => (
        <Chip
          active={selected === facet.value}
          count={facet.count}
          key={facet.value}
          label={facet.value}
          onPress={() =>
            onSelect(selected === facet.value ? null : facet.value)
          }
        />
      ))}
    </ScrollView>
  );
}

/** A composer's works grouped by form, with form, period and key chips. */
export function CompositionsSection({
  works,
}: {
  works: readonly ComposerWork[];
}) {
  const router = useRouter();
  const [filter, setFilter] = useState<WorkFilter>({});
  const facets = useMemo(() => composerWorkFacets(works), [works]);
  const groups = useMemo(
    () => groupWorksByForm(filterComposerWorks(works, filter)),
    [filter, works],
  );
  return (
    <View style={styles.section}>
      <ChipRail
        label="Form"
        onSelect={(form) => setFilter({ ...filter, form })}
        selected={filter.form}
        values={facets.forms}
      />
      <ChipRail
        label="Period"
        onSelect={(period) => setFilter({ ...filter, period })}
        selected={filter.period}
        values={facets.periods}
      />
      <ChipRail
        label="Key"
        onSelect={(key) => setFilter({ ...filter, key })}
        selected={filter.key}
        values={facets.keys}
      />
      {groups.length === 0 ? (
        <Text style={styles.empty}>No compositions match.</Text>
      ) : null}
      {groups.map((group) => (
        <View key={group.label} style={styles.group}>
          <Text accessibilityRole="header" style={styles.groupTitle}>
            {group.label}
          </Text>
          {group.works.map(({ work, recording_count }) => (
            <Pressable
              {...immediatePressFeedback}
              accessibilityRole="link"
              key={work.mbid}
              onPress={() => router.push(workRoute(work.mbid))}
              style={styles.workRow}
            >
              <View style={{ flex: 1 }}>
                <Text numberOfLines={2} style={styles.workTitle}>
                  {work.title}
                </Text>
                <Text numberOfLines={1} style={styles.workMeta}>
                  {joinMeta([
                    work.key,
                    work.catalogue,
                    work.composed,
                    work.period,
                  ])}
                </Text>
              </View>
              <Text style={styles.workCount}>
                {recording_count}{" "}
                {recording_count === 1 ? "recording" : "recordings"}
              </Text>
            </Pressable>
          ))}
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  heading: {
    marginTop: 14,
    marginHorizontal: 20,
    paddingLeft: 10,
    paddingVertical: 4,
    borderLeftWidth: 2,
    borderLeftColor: palette.borderStrong,
  },
  headingTitle: { color: "white", fontSize: 14, fontWeight: "800" },
  headingFacts: { color: palette.muted, fontSize: 12, marginTop: 2 },
  section: { gap: 8 },
  rail: { paddingHorizontal: 20, gap: 8, alignItems: "center" },
  railLabel: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 0.8,
    textTransform: "uppercase",
    width: 52,
  },
  chip: {
    borderRadius: 999,
    paddingHorizontal: 12,
    paddingVertical: 6,
    backgroundColor: "rgba(255,255,255,0.08)",
  },
  chipActive: { backgroundColor: "white" },
  chipText: { color: "#d4d4d8", fontSize: 12, fontWeight: "700" },
  chipTextActive: { color: "black" },
  group: { marginTop: 10 },
  groupTitle: {
    color: "white",
    fontSize: 17,
    fontWeight: "800",
    marginHorizontal: 20,
    marginBottom: 4,
  },
  workRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 10,
  },
  workTitle: { color: "white", fontSize: 14, fontWeight: "600" },
  workMeta: { color: palette.muted, fontSize: 12, marginTop: 2 },
  workCount: { color: palette.secondary, fontSize: 12 },
  empty: { color: palette.secondary, marginHorizontal: 20 },
});
