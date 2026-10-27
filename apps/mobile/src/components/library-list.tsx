import { ChevronRight, Server } from "lucide-react-native";
import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  Animated,
  Easing,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { palette } from "@/constants/colors";
import { libraryRowDetail, type LibraryRow } from "@/lib/known-libraries";

/** A softly pulsing dot shown while the network is being searched. */
export function SearchingIndicator({ label }: { label: string }) {
  const [pulse] = useState(() => new Animated.Value(0));
  useEffect(() => {
    const loop = Animated.loop(
      Animated.timing(pulse, {
        toValue: 1,
        duration: 1600,
        easing: Easing.out(Easing.quad),
        useNativeDriver: true,
      }),
    );
    loop.start();
    return () => loop.stop();
  }, [pulse]);
  return (
    <View accessibilityRole="progressbar" style={styles.searching}>
      <View style={styles.searchingDot}>
        <Animated.View
          style={[
            styles.searchingRing,
            {
              opacity: pulse.interpolate({
                inputRange: [0, 1],
                outputRange: [0.7, 0],
              }),
              transform: [
                {
                  scale: pulse.interpolate({
                    inputRange: [0, 1],
                    outputRange: [0.6, 2.4],
                  }),
                },
              ],
            },
          ]}
        />
        <View style={styles.searchingCore} />
      </View>
      <Text style={styles.searchingText}>{label}</Text>
    </View>
  );
}

export function LibraryList({
  rows,
  busy,
  connectingOrigin,
  onSelect,
  onForget,
}: {
  rows: LibraryRow[];
  busy: boolean;
  connectingOrigin?: string | null;
  onSelect: (row: LibraryRow) => void;
  onForget?: (row: LibraryRow) => void;
}) {
  return (
    <View style={styles.list}>
      {rows.map((row) => {
        const connecting = connectingOrigin === row.origin;
        return (
          <Pressable
            accessibilityHint={
              row.current
                ? "This is the library you are using"
                : "Switches to this Mujin Music library"
            }
            accessibilityLabel={`${row.name}, ${row.nearby ? "online" : "not seen nearby"}`}
            accessibilityRole="button"
            delayLongPress={350}
            disabled={busy}
            key={row.instanceId}
            onLongPress={
              onForget && row.known && !row.current
                ? () =>
                    Alert.alert(
                      `Forget ${row.name}?`,
                      "Its saved accounts are removed from this phone. You can connect again any time.",
                      [
                        { text: "Cancel", style: "cancel" },
                        {
                          text: "Forget",
                          style: "destructive",
                          onPress: () => onForget(row),
                        },
                      ],
                    )
                : undefined
            }
            onPress={() => onSelect(row)}
            style={({ pressed }) => [
              styles.card,
              row.current && styles.currentCard,
              pressed && styles.pressed,
              busy && !connecting && styles.dimmed,
            ]}
          >
            <View style={styles.icon}>
              <Server color="white" size={20} />
              <View
                style={[
                  styles.dot,
                  row.nearby ? styles.online : styles.offline,
                ]}
              />
            </View>
            <View style={styles.text}>
              <Text numberOfLines={1} style={styles.name}>
                {row.name}
              </Text>
              <Text numberOfLines={1} style={styles.detail}>
                {libraryRowDetail(row)}
              </Text>
            </View>
            {connecting ? (
              <ActivityIndicator color="white" size="small" />
            ) : row.current ? (
              <Text style={styles.badge}>In use</Text>
            ) : (
              <ChevronRight color={palette.muted} size={20} />
            )}
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  list: { gap: 10 },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: palette.border,
    backgroundColor: palette.elevatedStrong,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  currentCard: { borderColor: "rgba(163,201,139,0.45)" },
  pressed: { opacity: 0.75 },
  dimmed: { opacity: 0.55 },
  icon: {
    width: 42,
    height: 42,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(255,255,255,0.07)",
  },
  dot: {
    position: "absolute",
    right: -2,
    bottom: -2,
    width: 12,
    height: 12,
    borderRadius: 6,
    borderWidth: 2,
    borderColor: palette.elevatedStrong,
  },
  online: { backgroundColor: "#a3c98b" },
  offline: { backgroundColor: palette.muted },
  text: { flex: 1, minWidth: 0 },
  name: { color: "white", fontSize: 16, fontWeight: "800" },
  detail: { color: palette.secondary, fontSize: 13, marginTop: 3 },
  badge: {
    color: "#bcd8a9",
    fontSize: 12,
    fontWeight: "800",
    overflow: "hidden",
    borderRadius: 999,
    borderWidth: 1,
    borderColor: "rgba(163,201,139,0.4)",
    paddingHorizontal: 9,
    paddingVertical: 3,
  },
  searching: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 6,
  },
  searchingDot: {
    width: 14,
    height: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  searchingRing: {
    position: "absolute",
    width: 14,
    height: 14,
    borderRadius: 7,
    backgroundColor: "#a3c98b",
  },
  searchingCore: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: "#a3c98b",
  },
  searchingText: { color: palette.secondary, fontSize: 14 },
});
