import {
  catalogueHeadline,
  catalogueStatusText,
  composedLine,
  composerWorkFacets,
  filterComposerWorks,
  groupWorksByForm,
  notableWorks,
  workFacts,
  type CatalogueScope,
  type ComposerWork,
  type ComposerWorks,
  type WorkFacetValue,
  type WorkFilter,
  type WorkSummary,
} from "@parson/music-sdk";
import { useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { palette, tint } from "@/constants/colors";
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
  values: WorkFacetValue[];
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

const SCOPES: { key: CatalogueScope; label: string }[] = [
  { key: "all", label: "All works" },
  { key: "library", label: "In your library" },
  { key: "notable", label: "Notable" },
];

/** Rows per form before "Show all". */
const GROUP_PREVIEW = 6;

function LibraryState({ entry }: { entry: ComposerWork }) {
  return entry.track_count > 0 ? (
    <Text style={styles.owned}>
      {entry.recording_count || 1}{" "}
      {(entry.recording_count || 1) === 1 ? "recording" : "recordings"}
    </Text>
  ) : (
    <Text style={styles.notOwned}>Not in library</Text>
  );
}

/**
 * A composer's compositions: the whole MusicBrainz catalogue with what is
 * in the library marked, Wikidata's notable works first, then every piece
 * grouped by form with form, period and key chips.
 */
export function CompositionsSection({
  works,
  catalogue,
}: {
  works: readonly ComposerWork[];
  catalogue?: ComposerWorks["catalogue"];
}) {
  const router = useRouter();
  const [filter, setFilter] = useState<WorkFilter>({});
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const scope = filter.scope ?? "all";
  const inScope = useMemo(
    () => filterComposerWorks(works, { scope }),
    [scope, works],
  );
  const facets = useMemo(() => composerWorkFacets(inScope), [inScope]);
  const groups = useMemo(
    () => groupWorksByForm(filterComposerWorks(works, filter)),
    [filter, works],
  );
  const notable = useMemo(() => notableWorks(works, 8), [works]);
  const hasCatalogue = works.some((entry) => entry.in_catalogue);
  const status = catalogueStatusText(catalogue);
  const filtered = Boolean(filter.form || filter.period || filter.key);
  return (
    <View style={styles.section}>
      <View style={styles.summary}>
        <Text style={styles.summaryKicker}>
          {hasCatalogue ? "Complete catalogue" : "In your library"}
        </Text>
        <Text style={styles.summaryTitle}>
          {catalogueHeadline(works, catalogue)}
        </Text>
        {hasCatalogue ? (
          <Text style={styles.summaryNote}>
            Every work from MusicBrainz
            {catalogue?.notable ? "; notable works from Wikidata" : ""}.
          </Text>
        ) : null}
        {status ? <Text style={styles.summaryNote}>{status}</Text> : null}
      </View>
      {hasCatalogue ? (
        <ScrollView
          contentContainerStyle={styles.rail}
          horizontal
          showsHorizontalScrollIndicator={false}
        >
          {SCOPES.map((option) => (
            <Chip
              active={scope === option.key}
              key={option.key}
              label={option.label}
              onPress={() =>
                setFilter({ scope: option.key === "all" ? null : option.key })
              }
            />
          ))}
        </ScrollView>
      ) : null}
      {scope === "all" && !filtered && notable.length ? (
        <View>
          <Text accessibilityRole="header" style={styles.groupTitle}>
            Notable compositions
          </Text>
          <ScrollView
            contentContainerStyle={styles.cards}
            horizontal
            showsHorizontalScrollIndicator={false}
          >
            {notable.map((entry) => (
              <Pressable
                {...immediatePressFeedback}
                accessibilityRole="link"
                key={entry.work.mbid}
                onPress={() => router.push(workRoute(entry.work.mbid))}
                style={[styles.card, entry.track_count > 0 && styles.cardOwned]}
              >
                <Text numberOfLines={3} style={styles.cardTitle}>
                  {entry.work.title}
                </Text>
                <Text numberOfLines={1} style={styles.workMeta}>
                  {joinMeta([entry.work.form, composedLine(entry.work)])}
                </Text>
                <LibraryState entry={entry} />
              </Pressable>
            ))}
          </ScrollView>
        </View>
      ) : null}
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
      {groups.map((group) => {
        const open = filtered || expanded.has(group.label);
        const shown = open ? group.works : group.works.slice(0, GROUP_PREVIEW);
        return (
          <View key={group.label} style={styles.group}>
            <Text accessibilityRole="header" style={styles.groupTitle}>
              {group.label}{" "}
              <Text style={styles.groupCount}>{group.works.length}</Text>
            </Text>
            {shown.map((entry) => (
              <Pressable
                {...immediatePressFeedback}
                accessibilityRole="link"
                key={entry.work.mbid}
                onPress={() => router.push(workRoute(entry.work.mbid))}
                style={styles.workRow}
              >
                <View style={{ flex: 1 }}>
                  <Text
                    numberOfLines={2}
                    style={[
                      styles.workTitle,
                      entry.track_count === 0 && styles.workTitleMissing,
                    ]}
                  >
                    {entry.notable_rank != null ? "★ " : ""}
                    {entry.work.title}
                  </Text>
                  <Text numberOfLines={1} style={styles.workMeta}>
                    {joinMeta([
                      composedLine(entry.work),
                      entry.work.key,
                      entry.work.catalogue,
                      entry.work.period,
                    ])}
                  </Text>
                </View>
                <LibraryState entry={entry} />
              </Pressable>
            ))}
            {group.works.length > shown.length ? (
              <Pressable
                accessibilityRole="button"
                hitSlop={8}
                onPress={() =>
                  setExpanded((current) => new Set(current).add(group.label))
                }
              >
                <Text style={styles.more}>Show all {group.works.length}</Text>
              </Pressable>
            ) : null}
          </View>
        );
      })}
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
  headingTitle: { color: palette.text, fontSize: 14, fontWeight: "800" },
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
    backgroundColor: tint(0.08),
  },
  chipActive: { backgroundColor: palette.text },
  chipText: { color: palette.secondary, fontSize: 12, fontWeight: "700" },
  chipTextActive: { color: palette.background },
  group: { marginTop: 10 },
  groupTitle: {
    color: palette.text,
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
  workTitle: { color: palette.text, fontSize: 14, fontWeight: "600" },
  workMeta: { color: palette.muted, fontSize: 12, marginTop: 2 },
  workTitleMissing: { color: palette.secondary, fontWeight: "500" },
  owned: { color: palette.success, fontSize: 12, fontWeight: "700" },
  notOwned: { color: palette.muted, fontSize: 11 },
  groupCount: { color: palette.muted, fontSize: 13, fontWeight: "600" },
  more: {
    color: palette.secondary,
    fontSize: 12,
    fontWeight: "700",
    marginHorizontal: 20,
    marginTop: 4,
  },
  summary: {
    marginHorizontal: 20,
    padding: 14,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: tint(0.08),
    backgroundColor: tint(0.04),
    gap: 3,
  },
  summaryKicker: {
    color: palette.muted,
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  summaryTitle: { color: palette.text, fontSize: 15, fontWeight: "700" },
  summaryNote: { color: palette.muted, fontSize: 12, lineHeight: 17 },
  cards: { paddingHorizontal: 20, gap: 10 },
  card: {
    width: 190,
    minHeight: 112,
    padding: 12,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: tint(0.08),
    backgroundColor: tint(0.03),
    justifyContent: "space-between",
    gap: 6,
  },
  cardOwned: {
    borderColor: tint(0.16),
    backgroundColor: tint(0.07),
  },
  cardTitle: { color: palette.text, fontSize: 14, fontWeight: "700" },
  empty: { color: palette.secondary, marginHorizontal: 20 },
});
