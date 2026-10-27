import { Globe, House, ShieldCheck } from "lucide-react-native";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import type { KnownLibrary } from "@/lib/known-libraries";
import {
  checkRoute,
  routeCheckText,
  routeLabel,
  type RouteCheck,
} from "@/lib/route-check";

/**
 * Settings → Devices: how this phone reaches the current library (home
 * network or away from home) and a check of every away-from-home address,
 * run from wherever the phone is.
 */
export function AwayFromHome({
  library,
  origin,
  route,
}: {
  library: KnownLibrary | undefined;
  origin: string | null;
  route: "home" | "away" | null;
}) {
  const [checks, setChecks] = useState<Record<string, RouteCheck | "busy">>({});
  if (!library || !origin || !route) return null;
  const remote = library.remoteOrigins ?? [];
  const host = (() => {
    try {
      return new URL(origin).host;
    } catch {
      return origin;
    }
  })();

  async function runChecks() {
    if (!library) return;
    setChecks(Object.fromEntries(remote.map((item) => [item, "busy"])));
    const results = await Promise.all(
      remote.map(
        async (item) =>
          [item, await checkRoute(item, library.instanceId)] as const,
      ),
    );
    setChecks(Object.fromEntries(results));
  }

  const checking = Object.values(checks).includes("busy");

  return (
    <View style={styles.section}>
      <View style={styles.status}>
        <View style={styles.icon}>
          {route === "away" ? (
            <Globe color={palette.text} size={20} />
          ) : (
            <House color={palette.text} size={20} />
          )}
        </View>
        <View style={{ flex: 1 }}>
          <Text style={styles.title}>
            {route === "away" ? "Away from home" : "On your home network"}
          </Text>
          <Text numberOfLines={1} style={styles.detail}>
            {route === "away"
              ? `Connected through ${routeLabel(origin)} (${host})`
              : `Connected at ${host}`}
          </Text>
        </View>
      </View>
      {remote.length ? (
        <>
          <Text style={styles.caption}>
            When your home network is out of reach, Mujin Music switches to:
          </Text>
          {remote.map((item) => {
            const check = checks[item];
            return (
              <View key={item} style={styles.row}>
                <View
                  style={[
                    styles.light,
                    check && check !== "busy"
                      ? check.ok
                        ? route === "home"
                          ? null
                          : styles.good
                        : styles.bad
                      : null,
                  ]}
                />
                <View style={{ flex: 1 }}>
                  <Text style={styles.rowTitle}>{routeLabel(item)}</Text>
                  <Text numberOfLines={1} style={styles.detail}>
                    {check === "busy"
                      ? "Checking…"
                      : check
                        ? routeCheckText(check, {
                            onHomeNetwork: route === "home",
                          })
                        : item.replace(/^https?:\/\//, "")}
                  </Text>
                </View>
              </View>
            );
          })}
          <Pressable
            accessibilityRole="button"
            disabled={checking}
            onPress={() => void runChecks()}
            style={[styles.check, checking && { opacity: 0.5 }]}
          >
            <ShieldCheck color={palette.text} size={18} />
            <Text style={styles.checkText}>
              {checking ? "Checking from this phone…" : "Check from here"}
            </Text>
          </Pressable>
        </>
      ) : (
        <Text style={styles.caption}>
          This library is only reachable at home. Its owner can turn on access
          away from home (Tailscale, their own address or the router) in
          Settings → Connections on a computer.
        </Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  section: {
    marginTop: 18,
    padding: 16,
    borderRadius: 14,
    backgroundColor: palette.elevatedStrong,
    gap: 10,
  },
  status: { flexDirection: "row", alignItems: "center", gap: 13 },
  icon: {
    width: 40,
    height: 40,
    borderRadius: 12,
    backgroundColor: palette.surfaceStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  title: { color: palette.text, fontWeight: "800", fontSize: 15 },
  detail: { color: palette.secondary, fontSize: 13, marginTop: 2 },
  caption: { color: palette.secondary, fontSize: 13, lineHeight: 19 },
  row: { flexDirection: "row", alignItems: "center", gap: 12, minHeight: 40 },
  rowTitle: { color: palette.text, fontSize: 14, fontWeight: "600" },
  light: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: palette.muted,
    marginLeft: 4,
  },
  good: { backgroundColor: palette.success },
  bad: { backgroundColor: palette.danger },
  check: {
    marginTop: 4,
    height: 42,
    borderRadius: 21,
    borderWidth: 1,
    borderColor: palette.border,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  checkText: { color: palette.text, fontWeight: "700" },
});
