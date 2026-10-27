import {
  workDescription,
  type Composition,
  type FocusWork,
} from "@parson/music-sdk";
import { useRouter } from "expo-router";
import { ChevronDown, ChevronRight, Play } from "lucide-react-native";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Artwork } from "@/components/artwork";
import { palette } from "@/constants/colors";
import {
  durationLabel,
  recordingCaption,
  recordingSongs,
  workCaption,
} from "@/lib/library-focus";
import { usePlayer } from "@/providers/player-provider";

/** A composition with its recordings, expandable, as on a composer page. */
export function CompositionItem({
  composerId,
  composition,
  initiallyOpen = false,
}: {
  composerId?: string;
  composition: Composition;
  initiallyOpen?: boolean;
}) {
  const player = usePlayer();
  const [open, setOpen] = useState(initiallyOpen);
  const first = composition.recordings[0];
  const play = (index: number) => {
    const recording = composition.recordings[index];
    if (!recording) return;
    const songs = recordingSongs(recording);
    if (songs[0]) player.playSong(songs[0], songs);
  };
  const others = composition.composers.filter(
    (composer) => composer.id !== composerId,
  );
  const description = [
    others.map((composer) => composer.name).join(", "),
    workDescription(composition),
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <View style={styles.composition}>
      <View style={styles.compositionRow}>
        <Pressable
          accessibilityLabel={`Play ${composition.title}`}
          accessibilityRole="button"
          disabled={!first}
          hitSlop={8}
          onPress={() => play(0)}
          style={styles.playSmall}
        >
          <Play color="black" fill="black" size={14} />
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ expanded: open }}
          onPress={() => setOpen((value) => !value)}
          style={styles.compositionMain}
        >
          <View style={{ flex: 1 }}>
            <Text numberOfLines={2} style={styles.name}>
              {composition.title}
            </Text>
            {description ? (
              <Text numberOfLines={1} style={styles.description}>
                {description}
              </Text>
            ) : null}
            <Text numberOfLines={1} style={styles.meta}>
              {[
                `${composition.recordings.length} ${
                  composition.recordings.length === 1
                    ? "recording"
                    : "recordings"
                }`,
                composition.movementCount > 1
                  ? `${composition.movementCount} movements`
                  : null,
                composition.period,
              ]
                .filter(Boolean)
                .join(" · ")}
            </Text>
          </View>
          <ChevronDown
            color={palette.secondary}
            size={18}
            style={open ? { transform: [{ rotate: "180deg" }] } : undefined}
          />
        </Pressable>
      </View>
      {open
        ? composition.recordings.map((recording, index) => (
            <Pressable
              accessibilityLabel={`Play ${recording.albumName ?? "recording"}`}
              accessibilityRole="button"
              key={`${recording.albumId ?? "loose"}-${index}`}
              onPress={() => play(index)}
              style={styles.recording}
            >
              <Artwork path={recording.coverPath ?? ""} size={44} />
              <View style={{ flex: 1 }}>
                <Text numberOfLines={1} style={styles.recordingName}>
                  {recording.albumName ?? "Single track"}
                </Text>
                <Text numberOfLines={1} style={styles.meta}>
                  {recordingCaption(recording)}
                </Text>
                {recording.tracks.length > 1
                  ? recording.tracks.map((track, trackIndex) => (
                      <Text
                        key={track.id}
                        numberOfLines={1}
                        style={styles.movement}
                      >
                        {track.movementNumber ?? trackIndex + 1}.{" "}
                        {track.movement ?? track.title}{" "}
                        {durationLabel(track.durationSeconds)}
                      </Text>
                    ))
                  : null}
              </View>
              <Text style={styles.meta}>
                {durationLabel(recording.durationSeconds)}
              </Text>
            </Pressable>
          ))
        : null}
    </View>
  );
}

/** A row in the Compositions browser; opens the composition screen. */
export function WorkRow({ work }: { work: FocusWork }) {
  const router = useRouter();
  return (
    <Pressable
      accessibilityLabel={`${work.title}, composition`}
      accessibilityRole="button"
      onPress={() =>
        router.push(
          `/composition/${encodeURIComponent(work.id)}?title=${encodeURIComponent(work.title)}`,
        )
      }
      style={styles.workRow}
    >
      <View style={{ flex: 1 }}>
        <Text numberOfLines={2} style={styles.name}>
          {work.title}
        </Text>
        <Text numberOfLines={1} style={styles.description}>
          {workCaption(work)}
        </Text>
        <Text numberOfLines={1} style={styles.meta}>
          {[
            `${work.recordingCount} ${work.recordingCount === 1 ? "recording" : "recordings"}`,
            work.period,
          ]
            .filter(Boolean)
            .join(" · ")}
        </Text>
      </View>
      <ChevronRight color={palette.secondary} size={18} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  composition: {
    borderBottomColor: palette.elevated,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 20,
    paddingVertical: 10,
  },
  compositionRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  compositionMain: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
  },
  workRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 64,
    paddingHorizontal: 20,
    paddingVertical: 10,
    borderBottomColor: palette.elevated,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  playSmall: {
    width: 32,
    height: 32,
    borderRadius: 16,
    backgroundColor: "white",
    alignItems: "center",
    justifyContent: "center",
  },
  name: { color: "white", fontWeight: "700", fontSize: 15 },
  description: { color: "#d4d4d8", fontSize: 13, marginTop: 2 },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 2 },
  recording: {
    flexDirection: "row",
    gap: 12,
    alignItems: "flex-start",
    paddingVertical: 10,
    paddingLeft: 44,
  },
  recordingName: { color: "white", fontWeight: "600" },
  movement: { color: palette.muted, fontSize: 12, marginTop: 2 },
});
