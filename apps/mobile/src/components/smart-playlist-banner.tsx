import {
  detachSmartPlaylist,
  focusSummary,
  refreshSmartPlaylist,
  type SmartPlaylistInfo,
} from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Sparkles } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { palette } from "@/constants/colors";

/** Shows a smart playlist's focus and keeps its tracks current. */
export function SmartPlaylistBanner({
  playlistId,
  smart,
}: {
  playlistId: number;
  smart: SmartPlaylistInfo;
}) {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const refreshedOnOpen = useRef(false);
  const invalidate = async () => {
    await client.invalidateQueries({
      queryKey: ["playlist", String(playlistId)],
    });
    await client.invalidateQueries({ queryKey: ["playlists"] });
  };
  const refresh = async () => {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await refreshSmartPlaylist(playlistId);
      await invalidate();
    } catch {
      setError("Could not refresh this playlist.");
    } finally {
      setBusy(false);
    }
  };
  const refreshRef = useRef(refresh);
  useEffect(() => {
    refreshRef.current = refresh;
  });
  useEffect(() => {
    if (refreshedOnOpen.current) return;
    refreshedOnOpen.current = true;
    void refreshRef.current();
  }, []);

  return (
    <View style={styles.banner}>
      <Sparkles color="#f0dcc2" size={18} />
      <View style={{ flex: 1 }}>
        <Text style={styles.title}>Smart playlist</Text>
        <Text numberOfLines={2} style={styles.meta}>
          {focusSummary(smart.query) || "Every song in your library"}
        </Text>
        {error ? (
          <Text accessibilityRole="alert" style={styles.error}>
            {error}
          </Text>
        ) : null}
      </View>
      <Pressable
        accessibilityLabel="Refresh smart playlist"
        accessibilityRole="button"
        disabled={busy}
        onPress={() => void refresh()}
        style={styles.icon}
      >
        {busy ? (
          <ActivityIndicator color="white" size="small" />
        ) : (
          <RefreshCw color="white" size={18} />
        )}
      </Pressable>
      <Pressable
        accessibilityLabel="Stop updating from the focus"
        accessibilityRole="button"
        onPress={() =>
          void detachSmartPlaylist(playlistId).then(invalidate, () =>
            setError("Could not change this playlist."),
          )
        }
        style={styles.detach}
      >
        <Text style={styles.detachText}>Stop updating</Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    marginTop: 18,
    alignSelf: "stretch",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    padding: 14,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: palette.border,
    backgroundColor: "rgba(255,255,255,.04)",
  },
  title: { color: "white", fontWeight: "800" },
  meta: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  error: { color: "#f9b3af", fontSize: 12, marginTop: 4 },
  icon: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(255,255,255,.08)",
  },
  detach: { paddingVertical: 8 },
  detachText: { color: palette.secondary, fontSize: 12, fontWeight: "700" },
});
