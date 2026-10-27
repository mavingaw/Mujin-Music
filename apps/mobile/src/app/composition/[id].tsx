import {
  getFocusCompositions,
  scopedFocus,
  workScope,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
import { useMemo } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { CompositionItem } from "@/components/composition-item";
import { Screen } from "@/components/music-ui";
import { palette } from "@/constants/colors";
import { immediateBorderlessPressFeedback } from "@/lib/press-feedback";

/** One composition with every recording of it in the library. */
export default function CompositionScreen() {
  const { id, title } = useLocalSearchParams<{ id: string; title?: string }>();
  const router = useRouter();
  const query = useMemo(
    () => scopedFocus("songs", id ? [workScope(id, title)] : []),
    [id, title],
  );
  const data = useQuery({
    queryKey: ["library", "focus", "compositions", "mobile-work", id],
    queryFn: () => getFocusCompositions(query),
    enabled: Boolean(id),
  });
  const composition = data.data?.compositions[0];
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
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
        </View>
        <View style={styles.hero}>
          <Text style={styles.kicker}>COMPOSITION</Text>
          <Text style={styles.title}>{composition?.title ?? title ?? ""}</Text>
          {composition?.composers.length ? (
            <View style={styles.composers}>
              {composition.composers.map((composer) => (
                <Pressable
                  accessibilityRole="link"
                  key={composer.id}
                  onPress={() =>
                    router.push(`/composer/${encodeURIComponent(composer.id)}`)
                  }
                >
                  <Text style={styles.composer}>{composer.name}</Text>
                </Pressable>
              ))}
            </View>
          ) : null}
        </View>
        {data.isPending ? (
          <ActivityIndicator color={palette.text} style={{ marginTop: 40 }} />
        ) : !composition ? (
          <Pressable
            accessibilityRole="button"
            onPress={() => void data.refetch()}
            style={styles.center}
          >
            <Text style={styles.muted}>
              {data.isError
                ? "Could not load this composition · Tap to retry"
                : "No recordings of this composition."}
            </Text>
          </Pressable>
        ) : (
          <ScrollView contentContainerStyle={{ paddingBottom: 150 }}>
            <CompositionItem
              composerId={composition.composers[0]?.id}
              composition={composition}
              initiallyOpen
            />
          </ScrollView>
        )}
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: { height: 48, paddingHorizontal: 20, justifyContent: "center" },
  hero: { paddingHorizontal: 20, paddingBottom: 12 },
  kicker: {
    color: palette.secondary,
    fontSize: 12,
    fontWeight: "700",
    letterSpacing: 1.6,
  },
  title: {
    color: palette.text,
    fontSize: 28,
    fontWeight: "900",
    letterSpacing: -0.6,
    marginTop: 4,
  },
  composers: { flexDirection: "row", flexWrap: "wrap", gap: 10, marginTop: 6 },
  composer: { color: palette.text, fontWeight: "600", fontSize: 15 },
  muted: { color: palette.secondary, textAlign: "center" },
  center: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
  },
});
