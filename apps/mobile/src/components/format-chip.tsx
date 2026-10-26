import {
  describeAudioFormat,
  type AudioProperties,
  type AudioQualityTier,
} from "@parson/music-sdk";
import { StyleSheet, Text, View } from "react-native";

import { devicePlaybackCapabilities } from "@/lib/device-playback";
import { playsNatively } from "@/lib/playback-formats";

const TIER_COLORS: Record<AudioQualityTier, { border: string; text: string }> =
  {
    dsd: { border: "rgba(240,171,252,0.35)", text: "#f5d0fe" },
    hi_res: { border: "rgba(252,211,77,0.35)", text: "#fde68a" },
    lossless: { border: "rgba(125,211,252,0.3)", text: "#bae6fd" },
    lossy: { border: "rgba(255,255,255,0.14)", text: "#d4d4d8" },
    unknown: { border: "rgba(255,255,255,0.14)", text: "#d4d4d8" },
  };

/** Source format of the playing track, and the lossless form it arrives in. */
export function FormatChip({
  audio,
  path,
}: {
  audio?: AudioProperties | null;
  path: string;
}) {
  const summary = describeAudioFormat(audio);
  if (!summary) return null;
  const colors = TIER_COLORS[summary.tier];
  const converted = !playsNatively(path, audio, devicePlaybackCapabilities());
  const label =
    summary.tier === "dsd" || summary.badge === summary.name
      ? summary.badge
      : `${summary.name} ${summary.badge}`;
  const channels =
    summary.channels && !["stereo", "mono"].includes(summary.channels)
      ? ` · ${summary.channels}`
      : "";
  const delivery = converted
    ? summary.tier === "lossy"
      ? " → FLAC"
      : " → lossless FLAC"
    : "";
  return (
    <View
      accessibilityLabel={`${label}${channels}${converted ? ", converted to FLAC for this device" : ", original file"}`}
      style={[styles.chip, { borderColor: colors.border }]}
    >
      <Text style={[styles.text, { color: colors.text }]}>
        {summary.tier === "hi_res" ? "HI-RES " : ""}
        {label}
        {channels}
        <Text style={styles.delivery}>{delivery}</Text>
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  chip: {
    alignSelf: "flex-start",
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    marginTop: 8,
    paddingHorizontal: 9,
    paddingVertical: 3,
  },
  delivery: { color: "#a1a1aa", fontWeight: "500" },
  text: {
    fontSize: 11,
    fontVariant: ["tabular-nums"],
    fontWeight: "700",
    letterSpacing: 0.6,
  },
});
