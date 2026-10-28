import {
  releaseMatchSignals,
  releaseMatchSummary,
  UNVERIFIED_MATCH_TEXT,
  type ReleaseMatchCheck,
} from "@parson/music-sdk";
import { Check, CircleHelp, ShieldAlert, X } from "lucide-react-native";
import { Linking, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";

const AMBER = palette.brass;

const SIGNAL_ICON = {
  match: { Icon: Check, color: palette.success },
  partial: { Icon: CircleHelp, color: AMBER },
  mismatch: { Icon: X, color: palette.danger },
  unknown: { Icon: CircleHelp, color: palette.muted },
} as const;

/**
 * The closest MusicBrainz release failed the match check: the album shows
 * its own tags, and the evidence says why. Identifying the album is done
 * from Parson on the web.
 */
export function UnverifiedMatchNotice({
  check,
  text = UNVERIFIED_MATCH_TEXT,
}: {
  check: ReleaseMatchCheck;
  /** What the check means for this album's page. */
  text?: string;
}) {
  const summary = releaseMatchSummary(check);
  return (
    <View accessibilityLabel="Unverified match" style={styles.box}>
      <View style={styles.header}>
        <ShieldAlert color={AMBER} size={16} />
        <Text style={styles.eyebrow}>UNVERIFIED MATCH</Text>
      </View>
      <Text style={styles.release}>
        Closest release:{" "}
        <Text
          accessibilityRole="link"
          onPress={() => void Linking.openURL(check.url)}
          style={styles.link}
        >
          {check.release_title}
        </Text>
        {summary ? <Text style={styles.muted}> · {summary}</Text> : null}
      </Text>
      <Text style={styles.body}>{text}</Text>
      {releaseMatchSignals(check).map((signal) => {
        const { Icon, color } =
          SIGNAL_ICON[signal.outcome] ?? SIGNAL_ICON.unknown;
        return (
          <View key={signal.key} style={styles.signal}>
            <Icon color={color} size={13} style={styles.signalIcon} />
            <Text style={styles.signalText}>{signal.detail}</Text>
          </View>
        );
      })}
      <Text style={styles.hint}>
        An administrator can choose the right release with Identify album in
        Mujin Music on the web.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  box: {
    marginTop: 18,
    alignSelf: "stretch",
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(230,197,153,0.28)",
    backgroundColor: "rgba(230,197,153,0.07)",
    padding: 14,
    gap: 6,
  },
  header: { flexDirection: "row", alignItems: "center", gap: 7 },
  eyebrow: {
    color: AMBER,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.4,
  },
  release: { color: palette.text, fontSize: 14, lineHeight: 20 },
  link: { color: palette.text, fontWeight: "800" },
  muted: { color: palette.secondary },
  body: { color: palette.secondary, fontSize: 13, lineHeight: 19 },
  signal: { flexDirection: "row", alignItems: "flex-start", gap: 7 },
  signalIcon: { marginTop: 3 },
  signalText: {
    flex: 1,
    color: palette.secondary,
    fontSize: 13,
    lineHeight: 19,
  },
  hint: { color: palette.muted, fontSize: 12, lineHeight: 17, marginTop: 4 },
});
