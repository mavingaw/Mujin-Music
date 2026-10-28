import { useRouter } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { immediateBorderlessPressFeedback } from "@/lib/press-feedback";

/** Back button, kicker, big title and a summary line. */
export function BrowseHeader({
  kicker,
  title,
  subtitle,
  accent,
  right,
}: {
  kicker: string;
  title: string;
  subtitle?: string;
  accent?: string;
  right?: React.ReactNode;
}) {
  const router = useRouter();
  return (
    <View>
      <View style={styles.nav}>
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel="Back"
          accessibilityRole="button"
          hitSlop={12}
          onPress={router.back}
        >
          <ArrowLeft color={palette.text} size={25} />
        </Pressable>
        {right}
      </View>
      <View style={styles.hero}>
        <Text style={[styles.kicker, accent ? { color: accent } : null]}>
          {kicker}
        </Text>
        <Text accessibilityRole="header" style={styles.title}>
          {title}
        </Text>
        {subtitle ? <Text style={styles.meta}>{subtitle}</Text> : null}
      </View>
    </View>
  );
}

export function Chip({
  label,
  active,
  onPress,
}: {
  label: string;
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
      </Text>
    </Pressable>
  );
}

export const browseStyles = StyleSheet.create({
  chips: {
    flexDirection: "row",
    gap: 8,
    paddingHorizontal: 20,
    paddingBottom: 12,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 8,
  },
  rowText: { flex: 1, minWidth: 0 },
  rowTitle: { color: palette.text, fontSize: 15, fontWeight: "700" },
  rowMeta: { color: palette.secondary, fontSize: 13, marginTop: 3 },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "baseline",
    justifyContent: "space-between",
    paddingHorizontal: 20,
    paddingTop: 18,
    paddingBottom: 6,
    backgroundColor: palette.background,
  },
  sectionTitle: { color: palette.text, fontSize: 18, fontWeight: "800" },
  sectionMeta: { color: palette.muted, fontSize: 12 },
  empty: { alignItems: "center", padding: 40, marginTop: 24 },
  emptyTitle: { color: palette.text, fontSize: 18, fontWeight: "800" },
  emptyText: {
    color: palette.secondary,
    marginTop: 8,
    textAlign: "center",
    lineHeight: 20,
  },
});

const styles = StyleSheet.create({
  nav: {
    height: 50,
    paddingHorizontal: 20,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  hero: { paddingHorizontal: 20, paddingBottom: 14 },
  kicker: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  title: {
    color: palette.text,
    fontSize: 32,
    lineHeight: 36,
    fontWeight: "900",
    letterSpacing: -0.6,
    marginTop: 8,
  },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 6 },
  chip: {
    height: 34,
    paddingHorizontal: 14,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  chipActive: { backgroundColor: palette.text, borderColor: palette.text },
  chipText: { color: palette.text, fontSize: 13, fontWeight: "700" },
  chipTextActive: { color: palette.background },
});
