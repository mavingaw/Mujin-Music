import {
  getWork,
  groupRecordings,
  recordingPerformers,
  workFacts,
  type RecordingGrouping,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
import { useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import { Screen, SectionTitle, SongRow } from "@/components/music-ui";
import { EditorialBlock } from "@/components/credits";
import { Chip } from "@/components/works";
import { palette } from "@/constants/colors";
import { artistRoute, joinMeta, workRoute } from "@/lib/credit-links";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

const GROUPINGS: { key: RecordingGrouping; label: string }[] = [
  { key: "album", label: "Album" },
  { key: "conductor", label: "Conductor" },
  { key: "performer", label: "Performer" },
];

/** A composition and every recording of it in the library. */
export default function WorkScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const [grouping, setGrouping] = useState<RecordingGrouping>("album");
  const work = useQuery({
    queryKey: ["work", id],
    queryFn: () => getWork(id),
    enabled: !!id,
    staleTime: 60_000,
  });
  const groups = useMemo(
    () => (work.data ? groupRecordings(work.data.recordings, grouping) : []),
    [grouping, work.data],
  );
  if (work.isPending)
    return (
      <Screen>
        <ActivityIndicator color="white" style={{ flex: 1 }} />
      </Screen>
    );
  if (!work.data)
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          style={styles.errorState}
          onPress={() => void work.refetch()}
        >
          <Text style={styles.error}>Could not load work · Tap to retry</Text>
        </Pressable>
      </Screen>
    );
  const { work: piece, parent, parts, recordings } = work.data;
  const composer = piece.composer;
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
            <ArrowLeft color="white" size={25} />
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={{ paddingBottom: 135 }}>
          <View style={styles.hero}>
            <Text style={styles.kicker}>
              {parent ? "Movement" : "Composition"}
            </Text>
            <Text accessibilityRole="header" style={styles.title}>
              {piece.title}
            </Text>
            {composer ? (
              <Pressable
                {...immediatePressFeedback}
                accessibilityRole="link"
                disabled={!composer.artist_id}
                onPress={() =>
                  composer.artist_id &&
                  router.push(artistRoute(composer.artist_id))
                }
              >
                <Text style={styles.composer}>{composer.name}</Text>
              </Pressable>
            ) : null}
            {workFacts(piece).length ? (
              <Text style={styles.facts}>{workFacts(piece).join(" · ")}</Text>
            ) : null}
            {parent ? (
              <Pressable
                accessibilityRole="link"
                onPress={() => router.push(workRoute(parent.mbid))}
              >
                <Text style={styles.parent}>Part of {parent.title}</Text>
              </Pressable>
            ) : null}
          </View>
          {work.data.about ? (
            <View style={styles.block}>
              <SectionTitle>About</SectionTitle>
              <EditorialBlock text={work.data.about} />
            </View>
          ) : null}
          {parts.length ? (
            <View style={styles.block}>
              <SectionTitle>Movements</SectionTitle>
              {parts.map((part) => (
                <Text key={part.mbid} numberOfLines={1} style={styles.part}>
                  {part.title.startsWith(piece.title)
                    ? part.title
                        .slice(piece.title.length)
                        .replace(/^[:\s,–-]+/, "")
                    : part.title}
                </Text>
              ))}
            </View>
          ) : null}
          <SectionTitle>{`Recordings (${recordings.length})`}</SectionTitle>
          <View style={styles.groupings}>
            {GROUPINGS.map((entry) => (
              <Chip
                active={grouping === entry.key}
                key={entry.key}
                label={entry.label}
                onPress={() => setGrouping(entry.key)}
              />
            ))}
          </View>
          {groups.map((group) => (
            <View key={group.key}>
              {grouping !== "album" ? (
                <Text style={styles.groupTitle}>
                  {group.label} · {group.recordings.length}
                </Text>
              ) : null}
              {group.recordings.map((recording) => {
                const songs = recording.tracks.map((track) => track.song);
                return (
                  <View
                    key={`${group.key}-${recording.album.id}`}
                    style={styles.recording}
                  >
                    <Pressable
                      {...immediatePressFeedback}
                      accessibilityRole="link"
                      onPress={() =>
                        router.push(`/album/${recording.album.id}`)
                      }
                      style={styles.recordingHead}
                    >
                      <Artwork
                        path={recording.album.cover_url}
                        rounded={8}
                        size={64}
                      />
                      <View style={{ flex: 1 }}>
                        <Text numberOfLines={2} style={styles.albumName}>
                          {recording.album.name}
                        </Text>
                        <Text numberOfLines={2} style={styles.people}>
                          {recordingPerformers(recording) ||
                            recording.artist.name}
                        </Text>
                        <Text style={styles.meta}>
                          {joinMeta([
                            recording.year ? String(recording.year) : null,
                            parts.length > 1
                              ? recording.complete
                                ? "Complete"
                                : `${recording.tracks.length} of ${parts.length} movements`
                              : null,
                          ])}
                        </Text>
                      </View>
                    </Pressable>
                    {recording.tracks.map((track, index) => (
                      <SongRow
                        index={index}
                        key={track.song.id}
                        queue={songs}
                        showAlbum={false}
                        song={track.song}
                        title={track.part_title}
                        work={parent ?? piece}
                      />
                    ))}
                  </View>
                );
              })}
            </View>
          ))}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: {
    height: 50,
    paddingHorizontal: 20,
    flexDirection: "row",
    alignItems: "center",
  },
  hero: { paddingHorizontal: 20, paddingBottom: 12 },
  kicker: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  title: {
    color: "white",
    fontSize: 28,
    lineHeight: 32,
    fontWeight: "900",
    letterSpacing: -0.6,
    marginTop: 8,
  },
  composer: { color: "white", fontSize: 16, fontWeight: "700", marginTop: 10 },
  facts: { color: palette.secondary, fontSize: 13, marginTop: 6 },
  parent: { color: "#d4d4d8", fontSize: 13, marginTop: 8 },
  block: { marginBottom: 6 },
  part: {
    color: palette.secondary,
    fontSize: 14,
    marginHorizontal: 20,
    marginVertical: 3,
  },
  groupings: {
    flexDirection: "row",
    gap: 8,
    paddingHorizontal: 20,
    marginBottom: 8,
  },
  groupTitle: {
    color: "white",
    fontSize: 15,
    fontWeight: "800",
    marginHorizontal: 20,
    marginTop: 16,
  },
  recording: { marginTop: 14 },
  recordingHead: {
    flexDirection: "row",
    gap: 12,
    paddingHorizontal: 20,
    alignItems: "center",
  },
  albumName: { color: "white", fontSize: 15, fontWeight: "700" },
  people: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  meta: { color: palette.muted, fontSize: 12, marginTop: 2 },
  error: { color: palette.secondary, margin: 30 },
  errorState: { flex: 1, alignItems: "center", justifyContent: "center" },
});
