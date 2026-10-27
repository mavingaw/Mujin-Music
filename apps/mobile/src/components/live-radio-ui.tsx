import {
  liveStationSubtitle,
  removeLiveStation,
  saveLiveStation,
  type LiveQuality,
  type LiveStation,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Image } from "expo-image";
import { useRouter } from "expo-router";
import {
  BadgeCheck,
  ChevronRight,
  Heart,
  Play,
  Radio,
} from "lucide-react-native";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { useZonePlayback } from "@/hooks/use-zone-playback";
import { stationLogoUrl, stationSong } from "@/lib/live-radio";
import { immediatePressFeedback } from "@/lib/press-feedback";
import { usePlayer } from "@/providers/player-provider";
import { useSession } from "@/providers/session-provider";

const TIER_COLORS: Record<
  LiveQuality["tier"],
  { border: string; text: string; background: string }
> = {
  lossless: {
    border: "rgba(167,139,250,0.5)",
    text: "#ddd6fe",
    background: "rgba(139,92,246,0.16)",
  },
  high: {
    border: "rgba(56,189,248,0.4)",
    text: "#bae6fd",
    background: "rgba(14,165,233,0.12)",
  },
  standard: {
    border: "rgba(255,255,255,0.14)",
    text: "#d4d4d8",
    background: "rgba(255,255,255,0.05)",
  },
  low: {
    border: "rgba(251,191,36,0.4)",
    text: "#fde68a",
    background: "rgba(245,158,11,0.12)",
  },
  unknown: {
    border: "rgba(255,255,255,0.08)",
    text: "#71717a",
    background: "transparent",
  },
};

export function QualityTag({ quality }: { quality: LiveQuality }) {
  if (quality.tier === "unknown" && quality.label === "Unknown format")
    return null;
  const colors = TIER_COLORS[quality.tier];
  return (
    <View
      style={[
        styles.tag,
        { borderColor: colors.border, backgroundColor: colors.background },
      ]}
    >
      {quality.verified ? (
        <BadgeCheck color={colors.text} size={11} style={{ marginRight: 3 }} />
      ) : null}
      <Text style={[styles.tagText, { color: colors.text }]}>
        {quality.label}
      </Text>
    </View>
  );
}

export function LiveTag() {
  return (
    <View style={styles.live}>
      <View style={styles.liveDot} />
      <Text style={styles.liveText}>LIVE</Text>
    </View>
  );
}

function hue(name: string) {
  let value = 0;
  for (const character of name)
    value = (value * 31 + (character.codePointAt(0) ?? 0)) % 360;
  return value;
}

/** A station logo, contained, on a soft tile; a monogram when missing. */
export function StationLogo({
  station,
  size,
  rounded = 12,
}: {
  station: Pick<LiveStation, "name" | "logo_url">;
  size: number;
  rounded?: number;
}) {
  const session = useSession();
  const [failed, setFailed] = useState(false);
  const uri = failed ? null : stationLogoUrl(station, session.origin);
  const initials =
    station.name
      .replace(/[^\p{L}\p{N}\s]/gu, " ")
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0])
      .join("")
      .toUpperCase() || "FM";
  return (
    <View
      style={[
        styles.logoFrame,
        {
          width: size,
          height: size,
          borderRadius: rounded,
          backgroundColor: uri
            ? "#18181b"
            : `hsl(${hue(station.name)}, 55%, 32%)`,
        },
      ]}
    >
      {uri ? (
        <Image
          accessibilityIgnoresInvertColors
          contentFit="contain"
          onError={() => setFailed(true)}
          source={{ uri }}
          style={{ width: size * 0.78, height: size * 0.78 }}
        />
      ) : (
        <Text style={[styles.initials, { fontSize: size * 0.32 }]}>
          {initials}
        </Text>
      )}
    </View>
  );
}

/** Plays a station on the picked zone (or this phone). */
export function usePlayStation() {
  const playback = useZonePlayback();
  const session = useSession();
  const player = usePlayer();
  return {
    play: (station: LiveStation) => {
      if (player.current?.id === station.song_id) {
        player.toggle();
        return;
      }
      playback.playSong(stationSong(station, session.origin));
    },
    isCurrent: (station: LiveStation) => player.current?.id === station.song_id,
  };
}

export function useToggleSaved() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (station: LiveStation) => {
      if (station.saved) {
        await removeLiveStation(station.id);
        return { ...station, saved: false };
      }
      return saveLiveStation(station.id);
    },
    onSuccess: () =>
      void client.invalidateQueries({ queryKey: ["live-radio"] }),
  });
}

export function SaveButton({ station }: { station: LiveStation }) {
  const toggle = useToggleSaved();
  return (
    <Pressable
      accessibilityLabel={
        station.saved ? "Remove from My Live Radio" : "Add to My Live Radio"
      }
      accessibilityRole="button"
      accessibilityState={{ selected: station.saved }}
      disabled={toggle.isPending}
      hitSlop={10}
      onPress={() => toggle.mutate(station)}
      style={styles.iconButton}
    >
      <Heart
        color={station.saved ? "#fb7185" : palette.secondary}
        fill={station.saved ? "#fb7185" : "transparent"}
        size={20}
      />
    </Pressable>
  );
}

/** A station card for horizontal rails. */
export function StationTile({ station }: { station: LiveStation }) {
  const router = useRouter();
  const player = usePlayStation();
  const current = player.isCurrent(station);
  return (
    <Pressable
      accessibilityLabel={`Play ${station.name}`}
      accessibilityRole="button"
      onLongPress={() => router.push(`/radio/station/${station.id}`)}
      onPress={() => player.play(station)}
      {...immediatePressFeedback}
      style={styles.tile}
    >
      <View>
        <StationLogo size={136} station={station} />
        <View style={styles.tilePlay}>
          {current ? (
            <Radio color="black" size={16} />
          ) : (
            <Play color="black" fill="black" size={16} />
          )}
        </View>
      </View>
      <Text
        numberOfLines={1}
        style={[styles.tileName, current && styles.current]}
      >
        {station.name}
      </Text>
      <Text numberOfLines={1} style={styles.tileSubtitle}>
        {liveStationSubtitle(station)}
      </Text>
      <View style={styles.tileTags}>
        <QualityTag quality={station.quality} />
      </View>
    </Pressable>
  );
}

export function StationRail({ stations }: { stations: LiveStation[] }) {
  return (
    <ScrollView
      contentContainerStyle={styles.rail}
      horizontal
      showsHorizontalScrollIndicator={false}
    >
      {stations.map((station) => (
        <StationTile key={station.id} station={station} />
      ))}
    </ScrollView>
  );
}

/** A station row for lists and search results. */
export function StationListRow({ station }: { station: LiveStation }) {
  const router = useRouter();
  const player = usePlayStation();
  const current = player.isCurrent(station);
  return (
    <View style={styles.row}>
      <Pressable
        accessibilityLabel={`Play ${station.name}`}
        accessibilityRole="button"
        onPress={() => player.play(station)}
        {...immediatePressFeedback}
        style={styles.rowMain}
      >
        <StationLogo rounded={8} size={52} station={station} />
        <View style={styles.rowText}>
          <Text
            numberOfLines={1}
            style={[styles.rowName, current && styles.current]}
          >
            {station.name}
          </Text>
          <Text numberOfLines={1} style={styles.rowSubtitle}>
            {liveStationSubtitle(station)}
          </Text>
          <View style={styles.rowTags}>
            <QualityTag quality={station.quality} />
            {station.hls ? <Text style={styles.hls}>HLS</Text> : null}
          </View>
        </View>
      </Pressable>
      <SaveButton station={station} />
      <Pressable
        accessibilityLabel={`About ${station.name}`}
        accessibilityRole="button"
        hitSlop={8}
        onPress={() => router.push(`/radio/station/${station.id}`)}
        style={styles.iconButton}
      >
        <ChevronRight color={palette.muted} size={20} />
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  tag: {
    alignItems: "center",
    alignSelf: "flex-start",
    flexDirection: "row",
    borderRadius: 5,
    borderWidth: 1,
    paddingHorizontal: 6,
    paddingVertical: 1,
  },
  tagText: { fontSize: 10, fontWeight: "700", letterSpacing: 0.4 },
  live: {
    alignItems: "center",
    backgroundColor: "rgba(239,68,68,0.16)",
    borderRadius: 999,
    flexDirection: "row",
    gap: 4,
    paddingHorizontal: 7,
    paddingVertical: 2,
  },
  liveDot: { backgroundColor: "#f87171", borderRadius: 3, height: 6, width: 6 },
  liveText: {
    color: "#fca5a5",
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1.2,
  },
  logoFrame: {
    alignItems: "center",
    borderColor: "rgba(255,255,255,0.08)",
    borderWidth: StyleSheet.hairlineWidth,
    justifyContent: "center",
    overflow: "hidden",
  },
  initials: { color: "rgba(255,255,255,0.92)", fontWeight: "900" },
  iconButton: {
    alignItems: "center",
    height: 40,
    justifyContent: "center",
    width: 36,
  },
  tile: { width: 136 },
  tilePlay: {
    alignItems: "center",
    backgroundColor: "white",
    borderRadius: 18,
    bottom: 8,
    height: 36,
    justifyContent: "center",
    position: "absolute",
    right: 8,
    width: 36,
  },
  tileName: {
    color: palette.text,
    fontSize: 14,
    fontWeight: "700",
    marginTop: 8,
  },
  tileSubtitle: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  tileTags: { flexDirection: "row", marginTop: 6 },
  current: { color: "#6ee7b7" },
  rail: { gap: 14, paddingHorizontal: 20 },
  row: {
    alignItems: "center",
    flexDirection: "row",
    paddingHorizontal: 20,
    paddingVertical: 7,
  },
  rowMain: { alignItems: "center", flex: 1, flexDirection: "row", gap: 12 },
  rowText: { flex: 1, minWidth: 0 },
  rowName: { color: palette.text, fontSize: 15, fontWeight: "600" },
  rowSubtitle: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  rowTags: { alignItems: "center", flexDirection: "row", gap: 6, marginTop: 5 },
  hls: { color: palette.muted, fontSize: 10, fontWeight: "700" },
});
