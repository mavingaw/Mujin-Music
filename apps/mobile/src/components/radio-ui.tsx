import {
  radioComingUpIn,
  radioSkipReasons,
  radioStationLabel,
  type RadioSeed,
} from "@parson/music-sdk";
import { Radio, ThumbsDown, ThumbsUp, X } from "lucide-react-native";
import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Switch, Text, View } from "react-native";

import { Artwork } from "@/components/artwork";
import { DrawerAction } from "@/components/action-drawer";
import { palette } from "@/constants/colors";
import { immediateBorderlessPressFeedback } from "@/lib/press-feedback";
import { usePlayer, usePlayerPosition } from "@/providers/player-provider";

const sky = "#38bdf8";

const countdown = (seconds: number) =>
  `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;

/** Thumbs up / down for a queue item (current track or a Radio pick). */
export function RadioThumbs({
  index,
  name,
  size = 22,
  solid = false,
}: {
  index: number;
  name: string;
  size?: number;
  solid?: boolean;
}) {
  const player = usePlayer();
  const song = player.queue[index];
  const liked = Boolean(song && player.radio.liked.has(song.id));
  const current = index === player.currentIndex;
  return (
    <View style={styles.thumbs}>
      <Pressable
        {...immediateBorderlessPressFeedback}
        accessibilityLabel={
          liked ? `You liked ${name}` : `Thumbs up: more like ${name}`
        }
        accessibilityRole="button"
        accessibilityState={{ selected: liked }}
        hitSlop={8}
        style={solid ? [styles.solid, { backgroundColor: sky }] : styles.plain}
        onPress={() => player.radio.thumbsUp(index)}
      >
        <ThumbsUp
          color={solid ? "white" : liked ? sky : palette.secondary}
          fill={liked ? (solid ? "white" : sky) : "transparent"}
          size={size}
        />
      </Pressable>
      <Pressable
        {...immediateBorderlessPressFeedback}
        accessibilityLabel={
          current
            ? `Thumbs down: skip ${name} and play less like it`
            : `Thumbs down: replace ${name}`
        }
        accessibilityRole="button"
        hitSlop={8}
        style={
          solid ? [styles.solid, { backgroundColor: "white" }] : styles.plain
        }
        onPress={() => player.radio.thumbsDown(index)}
      >
        <ThumbsDown color={solid ? "black" : palette.secondary} size={size} />
      </Pressable>
    </View>
  );
}

/** Header of the Radio section at the end of the queue, with its switch. */
export function RadioQueueHeader({ hasPicks }: { hasPicks: boolean }) {
  const { radio } = usePlayer();
  return (
    <View style={styles.header}>
      <View style={{ flex: 1 }}>
        <View style={styles.headerTitleRow}>
          <Radio color={radio.enabled ? sky : palette.muted} size={18} />
          <Text style={styles.headerTitle}>Radio</Text>
        </View>
        <Text numberOfLines={1} style={styles.headerDetail}>
          {radio.enabled
            ? hasPicks
              ? radioStationLabel(radio.seed)
              : "Picking music based on your queue…"
            : "Off. Playback stops when your queue ends."}
        </Text>
      </View>
      <Switch
        accessibilityLabel="Radio: keep playing similar music when the queue ends"
        onValueChange={radio.setEnabled}
        thumbColor="white"
        trackColor={{ false: "#3f3f46", true: sky }}
        value={radio.enabled}
      />
    </View>
  );
}

/** Banner for the now-playing screen: "Coming up in 0:15" or a skip prompt. */
export function RadioNotices() {
  const { radio } = usePlayer();
  return radio.pendingSkip ? <RadioSkipPrompt /> : <RadioComingUp />;
}

function RadioComingUp() {
  const player = usePlayer();
  const { currentTime, duration } = usePlayerPosition();
  const [dismissed, setDismissed] = useState("");
  const nextIndex = player.currentIndex + 1;
  const next = player.queue[nextIndex];
  const remaining = radioComingUpIn({
    currentTime,
    duration,
    looping: player.repeat !== "none",
    nextIsRadioPick:
      player.radio.sectionStart >= 0 && player.radio.sectionStart <= nextIndex,
  });
  const key = next ? `${player.currentIndex}:${next.id}` : "";
  if (
    !next ||
    remaining === null ||
    !player.radio.enabled ||
    !player.radio.comingUpNotice ||
    dismissed === key
  )
    return null;
  return (
    <View accessibilityLiveRegion="polite" style={styles.card}>
      <Artwork path={next.album_object?.cover_url} size={46} rounded={6} />
      <View style={{ flex: 1 }}>
        <Text style={styles.cardKicker}>
          Coming up in {countdown(remaining)}
        </Text>
        <Text numberOfLines={1} style={styles.cardTitle}>
          {next.name}
        </Text>
        <Text numberOfLines={1} style={styles.cardDetail}>
          {next.artist}
        </Text>
      </View>
      <RadioThumbs index={nextIndex} name={next.name} size={18} solid />
      <Pressable
        {...immediateBorderlessPressFeedback}
        accessibilityLabel="Hide coming up"
        accessibilityRole="button"
        hitSlop={10}
        onPress={() => setDismissed(key)}
      >
        <X color={palette.secondary} size={18} />
      </Pressable>
    </View>
  );
}

function RadioSkipPrompt() {
  const { radio } = usePlayer();
  const { answerSkip, dismissSkip, pendingSkip } = radio;
  useEffect(() => {
    if (!pendingSkip) return;
    const timeout = setTimeout(dismissSkip, 12_000);
    return () => clearTimeout(timeout);
  }, [dismissSkip, pendingSkip]);
  if (!pendingSkip) return null;
  return (
    <View style={[styles.card, styles.skipCard]}>
      <View style={styles.skipHeader}>
        <Text numberOfLines={1} style={[styles.cardTitle, { flex: 1 }]}>
          Skipped {pendingSkip.name}. Tell us why?
        </Text>
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel="No reason"
          accessibilityRole="button"
          hitSlop={10}
          onPress={dismissSkip}
        >
          <X color={palette.secondary} size={18} />
        </Pressable>
      </View>
      <View style={styles.reasons}>
        {radioSkipReasons.map((reason) => (
          <Pressable
            accessibilityRole="button"
            key={reason.id}
            onPress={() => answerSkip(reason.id)}
            style={({ pressed }) => [
              styles.reason,
              pressed && { opacity: 0.6 },
            ]}
          >
            <Text style={styles.reasonText}>{reason.label}</Text>
          </Pressable>
        ))}
      </View>
    </View>
  );
}

/** Drawer action that starts a station from a song, album or artist. */
export function StartRadioAction({
  onStarted,
  seed,
}: {
  onStarted: () => void;
  seed: RadioSeed;
}) {
  const { radio } = usePlayer();
  const [state, setState] = useState<"idle" | "loading" | "empty" | "failed">(
    "idle",
  );
  return (
    <DrawerAction
      icon={Radio}
      label={
        state === "loading"
          ? "Starting radio…"
          : state === "empty"
            ? "Radio has nothing to add yet"
            : state === "failed"
              ? "Could not start radio · Try again"
              : "Start radio"
      }
      onPress={() => {
        if (state === "loading") return;
        setState("loading");
        void radio
          .start(seed)
          .then((station) => {
            if (station === null) {
              setState("empty");
              return;
            }
            setState("idle");
            onStarted();
          })
          .catch(() => setState("failed"));
      }}
    />
  );
}

/** Radio settings rows for the Playback settings tab. */
export function RadioSettingsRows() {
  const { radio } = usePlayer();
  return (
    <View style={styles.settings}>
      <Text style={styles.settingsTitle}>Radio</Text>
      <View style={styles.settingRow}>
        <View style={{ flex: 1 }}>
          <Text style={styles.settingLabel}>Continue with Radio</Text>
          <Text style={styles.settingDetail}>
            When your queue ends, keep playing similar music from your library.
          </Text>
        </View>
        <Switch
          accessibilityLabel="Continue with Radio"
          onValueChange={radio.setEnabled}
          thumbColor="white"
          trackColor={{ false: "#3f3f46", true: sky }}
          value={radio.enabled}
        />
      </View>
      <View style={styles.settingRow}>
        <View style={{ flex: 1 }}>
          <Text style={styles.settingLabel}>Coming up notice</Text>
          <Text style={styles.settingDetail}>
            Show the next Radio pick before it plays, with thumbs up and down.
          </Text>
        </View>
        <Switch
          accessibilityLabel="Coming up notice"
          onValueChange={radio.setComingUpNotice}
          thumbColor="white"
          trackColor={{ false: "#3f3f46", true: sky }}
          value={radio.comingUpNotice}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  thumbs: { flexDirection: "row", alignItems: "center", gap: 6 },
  plain: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  solid: {
    width: 38,
    height: 38,
    borderRadius: 19,
    alignItems: "center",
    justifyContent: "center",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    marginHorizontal: 20,
    marginTop: 14,
    marginBottom: 6,
    paddingTop: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: palette.borderStrong,
  },
  headerTitleRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  headerTitle: { color: "white", fontSize: 18, fontWeight: "900" },
  headerDetail: { color: palette.secondary, fontSize: 13, marginTop: 3 },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    padding: 10,
    borderRadius: 14,
    backgroundColor: "rgba(255,255,255,.09)",
  },
  skipCard: { flexDirection: "column", alignItems: "stretch" },
  skipHeader: { flexDirection: "row", alignItems: "center", gap: 8 },
  cardKicker: { color: sky, fontSize: 12, fontWeight: "800" },
  cardTitle: { color: "white", fontSize: 15, fontWeight: "800" },
  cardDetail: { color: palette.secondary, fontSize: 13 },
  reasons: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  reason: {
    borderRadius: 999,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  reasonText: { color: "white", fontSize: 13, fontWeight: "700" },
  settings: { marginTop: 28, gap: 4 },
  settingsTitle: { color: "white", fontSize: 18, fontWeight: "900" },
  settingRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: palette.border,
  },
  settingLabel: { color: "white", fontSize: 15, fontWeight: "700" },
  settingDetail: { color: palette.secondary, fontSize: 13, marginTop: 3 },
});
