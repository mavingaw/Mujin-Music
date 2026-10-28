import { getAlbumUpgradeStatus, isUpgradeOpen } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { alpha, palette } from "@/constants/colors";
import { upgradeChips } from "@/lib/upgrades";

/** Better versions status for an album: everyone sees it, nobody acts here. */
export function AlbumUpgradeStatus({ albumId }: { albumId: string }) {
  const status = useQuery({
    queryKey: ["upgrades", "album", albumId],
    queryFn: () => getAlbumUpgradeStatus(albumId),
    enabled: Boolean(albumId),
    refetchInterval: (query) =>
      query.state.data?.request && isUpgradeOpen(query.state.data.request.state)
        ? 5_000
        : false,
  });
  const chips = upgradeChips(status.data, palette);
  if (chips.length === 0) return null;
  return (
    <View style={styles.row}>
      {chips.map((chip) => (
        <View
          key={chip.key}
          style={[styles.chip, { borderColor: alpha(chip.color, 0.4) }]}
        >
          {chip.busy ? (
            <ActivityIndicator color={chip.color} size="small" />
          ) : (
            <View style={[styles.dot, { backgroundColor: chip.color }]} />
          )}
          <Text numberOfLines={1} style={[styles.label, { color: chip.color }]}>
            {chip.label}
          </Text>
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "center",
    gap: 8,
    marginTop: 10,
  },
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 5,
    maxWidth: "100%",
  },
  dot: { width: 6, height: 6, borderRadius: 3 },
  label: { fontSize: 12, fontWeight: "600", flexShrink: 1 },
});
