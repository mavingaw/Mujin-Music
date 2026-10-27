import { parseConnectLink } from "@parson/music-sdk";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef } from "react";
import { ActivityIndicator, StyleSheet, View } from "react-native";

import { useSession } from "@/providers/session-provider";
import { palette } from "@/constants/colors";

/**
 * Handles `parson://connect?server=…&name=…&id=…`, the link in the QR code a
 * Parson server shows under Settings → Connections. Opens that library
 * straight away; a library this phone already knows signs back in with its
 * saved account.
 */
export default function ConnectLinkScreen() {
  const params = useLocalSearchParams<{
    server?: string;
    name?: string;
    id?: string;
  }>();
  const router = useRouter();
  const session = useSession();
  const handled = useRef(false);
  useEffect(() => {
    if (handled.current) return;
    handled.current = true;
    const query = new URLSearchParams();
    for (const key of ["server", "name", "id"] as const) {
      const value = params[key];
      if (typeof value === "string") query.set(key, value);
    }
    const link = parseConnectLink(`parson://connect?${query.toString()}`);
    router.replace("/");
    if (!link) return;
    if (
      link.instanceId &&
      link.instanceId === session.instanceId &&
      session.phase === "ready"
    )
      return;
    void session.switchLibrary(link.origin);
  }, [params, router, session]);
  return (
    <View style={styles.page}>
      <ActivityIndicator color={palette.text} />
    </View>
  );
}

const styles = StyleSheet.create({
  page: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: palette.background,
  },
});
