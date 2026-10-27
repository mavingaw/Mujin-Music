import {
  focusResultLabel,
  removeFocusChip,
  toggleFocusValue,
  type FocusChip,
  type FocusGroup,
  type FocusSelection,
  type FocusSummary,
} from "@parson/music-sdk";
import { Check, ListFilter, X } from "lucide-react-native";
import { useState } from "react";
import {
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { palette, tint } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";

const GOLD = palette.brass;

function GroupList({
  group,
  onToggle,
}: {
  group: FocusGroup;
  onToggle: (chip: FocusChip) => void;
}) {
  return (
    <View style={styles.group}>
      <Text style={styles.groupLabel}>{group.label}</Text>
      {group.values.map((chip) => {
        const empty = chip.count === 0 && !chip.selected;
        return (
          <Pressable
            {...immediatePressFeedback}
            accessibilityRole="checkbox"
            accessibilityState={{ checked: chip.selected, disabled: empty }}
            disabled={empty}
            key={chip.key}
            onPress={() => onToggle(chip)}
            style={[
              styles.value,
              chip.selected && styles.valueSelected,
              empty && { opacity: 0.4 },
            ]}
          >
            <View style={[styles.box, chip.selected && styles.boxSelected]}>
              {chip.selected ? (
                <Check color={palette.background} size={12} strokeWidth={3} />
              ) : null}
            </View>
            <Text numberOfLines={1} style={styles.valueLabel}>
              {chip.label}
            </Text>
            <Text style={styles.count}>{chip.count}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

/**
 * Focus on a phone: chosen filters as a row of chips, a Hi-Res toggle,
 * and a sheet with every group (performer, conductor, ensemble, soloist,
 * instrument, composer, form, period, bit depth, sample rate, source).
 */
export function FocusBar({
  summary,
  selection,
  onChange,
  loading = false,
  noun = { one: "album", other: "albums" },
  note,
}: {
  summary: FocusSummary | null | undefined;
  selection: FocusSelection;
  onChange: (selection: FocusSelection) => void;
  loading?: boolean;
  noun?: { one: string; other: string };
  note?: string;
}) {
  const [open, setOpen] = useState(false);
  if (!summary || summary.total === 0) return null;
  const hires = summary.groups
    .find((group) => group.key === "quality")
    ?.values.find((chip) => chip.key === "hires");
  const active = summary.selected.length;
  const filterable = summary.groups.length > 0;
  return (
    <View style={styles.bar}>
      <ScrollView
        contentContainerStyle={styles.chips}
        horizontal
        showsHorizontalScrollIndicator={false}
      >
        <Pressable
          {...immediatePressFeedback}
          accessibilityLabel={`Focus, ${active} chosen`}
          accessibilityRole="button"
          disabled={!filterable}
          onPress={() => setOpen(true)}
          style={[
            styles.focus,
            active ? styles.focusActive : null,
            !filterable && { opacity: 0.5 },
          ]}
        >
          <ListFilter
            color={active ? palette.background : palette.text}
            size={15}
          />
          <Text
            style={[
              styles.focusText,
              active ? { color: palette.background } : null,
            ]}
          >
            Focus{active ? ` · ${active}` : ""}
          </Text>
        </Pressable>
        {hires ? (
          <Pressable
            {...immediatePressFeedback}
            accessibilityRole="switch"
            accessibilityState={{ checked: hires.selected }}
            onPress={() =>
              onChange(toggleFocusValue(selection, "quality", "hires"))
            }
            style={[styles.hires, hires.selected && styles.hiresOn]}
          >
            <Text
              style={[
                styles.hiresText,
                hires.selected && { color: palette.background },
              ]}
            >
              HI-RES ONLY · {hires.count}
            </Text>
          </Pressable>
        ) : null}
        {summary.selected
          .filter((chip) => chip.key !== "quality=hires")
          .map((chip) => (
            <Pressable
              {...immediatePressFeedback}
              accessibilityLabel={`Remove ${chip.label}`}
              accessibilityRole="button"
              key={chip.key}
              onPress={() => onChange(removeFocusChip(selection, chip.key))}
              style={styles.selected}
            >
              <Text numberOfLines={1} style={styles.selectedText}>
                {chip.label}
              </Text>
              <X color={palette.secondary} size={13} />
            </Pressable>
          ))}
      </ScrollView>
      <Text style={styles.result}>
        {loading ? "Updating… " : ""}
        {focusResultLabel(summary, noun)}
      </Text>
      <Modal
        animationType="slide"
        onRequestClose={() => setOpen(false)}
        transparent
        visible={open}
      >
        <View style={styles.backdrop}>
          <SafeAreaView edges={["bottom"]} style={styles.sheet}>
            <View style={styles.sheetHeader}>
              <Text style={styles.sheetTitle}>Focus</Text>
              <Text style={styles.sheetResult}>
                {focusResultLabel(summary, noun)}
              </Text>
              {active ? (
                <Pressable
                  accessibilityRole="button"
                  hitSlop={8}
                  onPress={() => onChange({})}
                >
                  <Text style={styles.clear}>Clear</Text>
                </Pressable>
              ) : null}
              <Pressable
                accessibilityRole="button"
                hitSlop={8}
                onPress={() => setOpen(false)}
                style={styles.done}
              >
                <Text style={styles.doneText}>Done</Text>
              </Pressable>
            </View>
            <ScrollView contentContainerStyle={{ paddingBottom: 24 }}>
              {summary.groups.map((group) => (
                <GroupList
                  group={group}
                  key={group.key}
                  onToggle={(chip) =>
                    onChange(toggleFocusValue(selection, group.key, chip.key))
                  }
                />
              ))}
              {note ? <Text style={styles.note}>{note}</Text> : null}
            </ScrollView>
          </SafeAreaView>
        </View>
      </Modal>
    </View>
  );
}

const styles = StyleSheet.create({
  bar: { marginBottom: 14 },
  chips: { paddingHorizontal: 20, gap: 8, alignItems: "center" },
  focus: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    height: 34,
    borderRadius: 17,
    paddingHorizontal: 14,
    backgroundColor: palette.elevatedStrong,
    borderWidth: 1,
    borderColor: palette.border,
  },
  focusActive: { backgroundColor: palette.text, borderColor: palette.text },
  focusText: { color: palette.text, fontWeight: "700", fontSize: 13 },
  hires: {
    height: 34,
    borderRadius: 17,
    paddingHorizontal: 12,
    justifyContent: "center",
    borderWidth: 1,
    borderColor: "rgba(230,197,153,0.4)",
  },
  hiresOn: { backgroundColor: GOLD, borderColor: GOLD },
  hiresText: {
    color: GOLD,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
  selected: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    height: 34,
    maxWidth: 220,
    borderRadius: 17,
    paddingLeft: 12,
    paddingRight: 10,
    backgroundColor: tint(0.12),
  },
  selectedText: { color: palette.text, fontSize: 13, fontWeight: "600" },
  result: {
    color: palette.secondary,
    fontSize: 12,
    marginTop: 8,
    paddingHorizontal: 20,
  },
  backdrop: {
    flex: 1,
    justifyContent: "flex-end",
    backgroundColor: "rgba(0,0,0,0.6)",
  },
  sheet: {
    maxHeight: "82%",
    backgroundColor: "#0e0e10",
    borderTopLeftRadius: 22,
    borderTopRightRadius: 22,
    borderWidth: 1,
    borderColor: palette.border,
  },
  sheetHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 16,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: palette.border,
  },
  sheetTitle: { color: palette.text, fontSize: 18, fontWeight: "800" },
  sheetResult: { color: palette.secondary, fontSize: 13, flex: 1 },
  clear: { color: palette.secondary, fontWeight: "700" },
  done: {
    backgroundColor: palette.text,
    borderRadius: 14,
    paddingHorizontal: 12,
    paddingVertical: 6,
  },
  doneText: { color: palette.background, fontWeight: "800" },
  group: { paddingHorizontal: 20, paddingTop: 18 },
  groupLabel: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    marginBottom: 6,
    textTransform: "uppercase",
  },
  value: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 40,
    paddingHorizontal: 8,
    borderRadius: 10,
  },
  valueSelected: { backgroundColor: tint(0.1) },
  box: {
    width: 18,
    height: 18,
    borderRadius: 5,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  boxSelected: { backgroundColor: palette.text, borderColor: palette.text },
  valueLabel: { color: palette.text, fontSize: 15, flex: 1 },
  count: { color: palette.secondary, fontSize: 13 },
  note: {
    color: palette.muted,
    fontSize: 12,
    lineHeight: 17,
    paddingHorizontal: 20,
    paddingTop: 18,
  },
});
