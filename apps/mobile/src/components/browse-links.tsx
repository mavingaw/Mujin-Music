import { useRouter, type Href } from "expo-router";
import { Clock, Hash, History, type LucideIcon } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";

const LINKS: { href: Href; label: string; icon: LucideIcon; tint: string }[] = [
  {
    href: "/listen-later",
    label: "Listen Later",
    icon: Clock,
    tint: palette.accent,
  },
  { href: "/tags", label: "Tags", icon: Hash, tint: palette.accent },
  { href: "/history", label: "History", icon: History, tint: palette.accent },
];

/** Roon's BROWSE shortcuts: Listen Later, Tags and History. */
export function BrowseLinks() {
  const router = useRouter();
  return (
    <View style={styles.row}>
      {LINKS.map(({ href, label, icon: Icon, tint }) => (
        <Pressable
          {...immediatePressFeedback}
          accessibilityRole="link"
          key={label}
          onPress={() => router.push(href)}
          style={styles.card}
        >
          <Icon color={tint} size={18} strokeWidth={2.2} />
          <Text numberOfLines={1} style={styles.label}>
            {label}
          </Text>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    gap: 8,
    paddingHorizontal: 20,
    marginBottom: 12,
  },
  card: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    height: 44,
    paddingHorizontal: 12,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.border,
    backgroundColor: palette.elevated,
  },
  label: { flexShrink: 1, color: "white", fontSize: 13, fontWeight: "700" },
});
