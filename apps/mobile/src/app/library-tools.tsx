import {
  cancelToolJob,
  checkForFakeLossless,
  checkSkippedFiles,
  getActiveUpgradeRequests,
  upgradeProgressDetail,
  upgradeStatusLabel,
  getLibraryToolsOverview,
  getSkippedFiles,
  getToolJobs,
  ignoreSkippedFiles,
  reanalyzeLibraryItems,
  resolveSuggestedDuplicates,
  toolJobProgress,
  type ToolJob,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import {
  ArrowLeft,
  AudioWaveform,
  EyeOff,
  FolderSearch,
  Sparkles,
  X,
} from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { PageTitle, Screen, SectionTitle } from "@/components/music-ui";
import { layout, palette } from "@/constants/colors";
import {
  formatBytes,
  formatPlayingTime,
  jobLine,
  losslessPercent,
  skippedByReason,
  tierColor,
  tierLabel,
} from "@/lib/library-tools";
import { immediatePressFeedback } from "@/lib/press-feedback";
import { useSession } from "@/providers/session-provider";

const ACCENT = palette.accent;

/**
 * Library tools on the phone: statistics, quality breakdown, skipped files
 * and the long jobs, for an admin away from the desktop. The full editing
 * tools (Albumizer, versions) live in Parson on the web and desktop.
 */
export default function LibraryToolsScreen() {
  const router = useRouter();
  const client = useQueryClient();
  const session = useSession();
  const admin = session.claims?.role === "admin";
  const [message, setMessage] = useState("");
  const overview = useQuery({
    queryKey: ["library-tools", "overview"],
    queryFn: getLibraryToolsOverview,
    enabled: admin,
  });
  const skipped = useQuery({
    queryKey: ["library-tools", "skipped"],
    queryFn: getSkippedFiles,
    enabled: admin,
  });
  const jobs = useQuery({
    queryKey: ["library-tools", "jobs"],
    queryFn: getToolJobs,
    enabled: admin,
    refetchInterval: (query) =>
      query.state.data?.some((job) => job.state === "running") ? 1_000 : false,
  });
  const upgrades = useQuery({
    queryKey: ["upgrades", "active"],
    queryFn: getActiveUpgradeRequests,
    enabled: admin,
    refetchInterval: (query) =>
      (query.state.data?.length ?? 0) > 0 ? 5_000 : false,
  });
  const running = (jobs.data ?? []).filter((job) => job.state === "running");
  const runningCount = running.length;
  const previous = useRef(runningCount);
  useEffect(() => {
    if (previous.current > runningCount) {
      void client.invalidateQueries({ queryKey: ["library-tools"] });
    }
    previous.current = runningCount;
  }, [client, runningCount]);

  const start = async (work: () => Promise<unknown>, failure: string) => {
    setMessage("");
    try {
      await work();
      await client.invalidateQueries({ queryKey: ["library-tools"] });
    } catch {
      setMessage(failure);
    }
  };

  const stats = overview.data?.stats;
  const tiersTotal =
    stats?.tiers.reduce((sum, tier) => sum + tier.tracks, 0) ?? 0;
  const groups = skippedByReason(skipped.data?.items ?? []);

  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <ScrollView
          contentContainerStyle={{ paddingBottom: layout.miniPlayer + 40 }}
        >
          <Pressable
            accessibilityLabel="Back"
            accessibilityRole="button"
            onPress={() => router.back()}
            style={styles.back}
          >
            <ArrowLeft color="white" size={22} />
          </Pressable>
          <PageTitle subtitle="Mujin Music only reads your music files; every change can be undone.">
            Library tools
          </PageTitle>
          {!admin ? (
            <Text style={styles.note}>
              Library tools are for the admin of this server.
            </Text>
          ) : !stats ? (
            <ActivityIndicator color="white" style={{ marginTop: 40 }} />
          ) : (
            <>
              {running.map((job) => (
                <JobRow job={job} key={job.id} />
              ))}
              <View style={styles.statGrid}>
                <Stat label="Tracks" value={stats.tracks.toLocaleString()} />
                <Stat label="Albums" value={stats.albums.toLocaleString()} />
                <Stat
                  label="Playing time"
                  value={formatPlayingTime(stats.duration_seconds)}
                />
                <Stat label="On disk" value={formatBytes(stats.size_bytes)} />
              </View>

              <SectionTitle>Quality</SectionTitle>
              <View style={styles.card}>
                <Text style={styles.cardTitle}>
                  <Text style={{ color: palette.accentSoft }}>
                    {losslessPercent(stats.tiers)}%
                  </Text>{" "}
                  lossless
                </Text>
                <View style={styles.bar}>
                  {stats.tiers.map((tier) => (
                    <View
                      key={tier.key}
                      style={{
                        flex: Math.max(
                          tier.tracks / Math.max(1, tiersTotal),
                          0.01,
                        ),
                        backgroundColor: tierColor(tier.key),
                      }}
                    />
                  ))}
                </View>
                {stats.tiers.map((tier) => (
                  <View key={tier.key} style={styles.legendRow}>
                    <View
                      style={[
                        styles.dot,
                        { backgroundColor: tierColor(tier.key) },
                      ]}
                    />
                    <Text style={styles.legendLabel}>
                      {tierLabel(tier.key, tier.label)}
                    </Text>
                    <Text style={styles.legendValue}>
                      {tier.tracks.toLocaleString()}
                    </Text>
                  </View>
                ))}
                <Text style={styles.subtle}>
                  {stats.formats
                    .slice(0, 5)
                    .map((format) => `${format.label} ${format.tracks}`)
                    .join(" · ")}
                </Text>
              </View>

              <SectionTitle>Needs attention</SectionTitle>
              <View style={styles.card}>
                <Attention
                  label="Skipped files"
                  value={overview.data?.skipped ?? 0}
                />
                <Attention
                  label="Duplicates"
                  value={overview.data?.duplicates ?? 0}
                />
                <Attention
                  label="Loose tracks"
                  value={overview.data?.loose_tracks ?? 0}
                />
                <Attention
                  label="Missing files"
                  value={overview.data?.missing ?? 0}
                />
                <Attention
                  label="Not analyzed"
                  value={stats.analysis.awaiting}
                />
              </View>

              <View style={styles.actions}>
                <Action
                  icon={<FolderSearch color="black" size={18} />}
                  label="Check folders"
                  onPress={() =>
                    void start(
                      checkSkippedFiles,
                      "Could not check the folders.",
                    )
                  }
                  primary
                />
                <Action
                  icon={<AudioWaveform color="white" size={18} />}
                  label="Analyze new tracks"
                  onPress={() =>
                    void start(
                      () => reanalyzeLibraryItems({ scope: "library" }),
                      "Could not start the analysis.",
                    )
                  }
                />
                {(overview.data?.duplicates_suggested ?? 0) > 0 && (
                  <Action
                    icon={<Sparkles color="white" size={18} />}
                    label={`Keep best copies (${overview.data?.duplicates_suggested})`}
                    onPress={() =>
                      void start(
                        () => resolveSuggestedDuplicates("merge"),
                        "Could not tidy the duplicates.",
                      )
                    }
                  />
                )}
              </View>
              {message ? <Text style={styles.error}>{message}</Text> : null}

              <SectionTitle>Better versions</SectionTitle>
              <View style={styles.card}>
                {(upgrades.data ?? []).length === 0 ? (
                  <Text style={styles.subtle}>
                    No lossless copies on their way. Ask FETCH for them in
                    Library tools → Better versions on the web or desktop.
                  </Text>
                ) : (
                  (upgrades.data ?? []).map((request) => (
                    <View key={request.id} style={styles.fileRow}>
                      <View style={{ flex: 1 }}>
                        <Text numberOfLines={1} style={styles.fileName}>
                          {request.album_title}{" "}
                          <Text style={styles.subtle}>
                            {request.artist_name}
                          </Text>
                        </Text>
                        <Text numberOfLines={1} style={styles.upgradeLine}>
                          {upgradeStatusLabel(request)}
                          {upgradeProgressDetail(request)
                            ? ` · ${upgradeProgressDetail(request)}`
                            : ""}
                        </Text>
                      </View>
                    </View>
                  ))
                )}
              </View>
              <View style={styles.actions}>
                <Action
                  icon={<AudioWaveform color="white" size={18} />}
                  label="Check for fake lossless"
                  onPress={() =>
                    void start(
                      () => checkForFakeLossless(),
                      "Could not start the check.",
                    )
                  }
                />
              </View>

              {groups.length > 0 && <SectionTitle>Skipped files</SectionTitle>}
              {groups.map((group) => (
                <View key={group.label} style={styles.card}>
                  <Text style={styles.cardTitle}>
                    {group.label}{" "}
                    <Text style={styles.subtle}>{group.items.length}</Text>
                  </Text>
                  {group.items.slice(0, 20).map((item) => (
                    <View key={item.path} style={styles.fileRow}>
                      <View style={{ flex: 1 }}>
                        <Text numberOfLines={1} style={styles.fileName}>
                          {item.file_name}
                        </Text>
                        <Text numberOfLines={2} style={styles.subtle}>
                          {item.fix.summary}
                        </Text>
                      </View>
                      <Pressable
                        {...immediatePressFeedback}
                        accessibilityLabel={`Ignore ${item.file_name}`}
                        accessibilityRole="button"
                        hitSlop={10}
                        onPress={() =>
                          void start(
                            () => ignoreSkippedFiles([item.path]),
                            "Could not update the list.",
                          )
                        }
                      >
                        <EyeOff color={palette.secondary} size={18} />
                      </Pressable>
                    </View>
                  ))}
                </View>
              ))}
            </>
          )}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

function JobRow({ job }: { job: ToolJob }) {
  const progress = toolJobProgress(job);
  return (
    <View style={styles.job}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 10 }}>
        <ActivityIndicator color={ACCENT} size="small" />
        <View style={{ flex: 1 }}>
          <Text numberOfLines={1} style={styles.fileName}>
            {job.title} · {job.scope}
          </Text>
          <Text numberOfLines={1} style={styles.subtle}>
            {jobLine(job)}
          </Text>
        </View>
        <Pressable
          accessibilityLabel={`Stop ${job.title}`}
          accessibilityRole="button"
          hitSlop={10}
          onPress={() => void cancelToolJob(job.id)}
        >
          <X color={palette.secondary} size={18} />
        </Pressable>
      </View>
      <View style={styles.progressTrack}>
        <View
          style={[
            styles.progressFill,
            { width: `${Math.round((progress ?? 0.15) * 100)}%` },
          ]}
        />
      </View>
    </View>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.stat}>
      <Text style={styles.statLabel}>{label.toUpperCase()}</Text>
      <Text numberOfLines={1} style={styles.statValue}>
        {value}
      </Text>
    </View>
  );
}

function Attention({ label, value }: { label: string; value: number }) {
  return (
    <View style={styles.legendRow}>
      <View
        style={[
          styles.dot,
          { backgroundColor: value > 0 ? "#f5c542" : palette.success },
        ]}
      />
      <Text style={styles.legendLabel}>{label}</Text>
      <Text style={styles.legendValue}>{value.toLocaleString()}</Text>
    </View>
  );
}

function Action({
  icon,
  label,
  onPress,
  primary = false,
}: {
  icon: React.ReactNode;
  label: string;
  onPress: () => void;
  primary?: boolean;
}) {
  return (
    <Pressable
      {...immediatePressFeedback}
      accessibilityRole="button"
      onPress={onPress}
      style={[styles.action, primary && styles.actionPrimary]}
    >
      {icon}
      <Text style={[styles.actionText, primary && { color: "black" }]}>
        {label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  back: {
    paddingHorizontal: layout.horizontal,
    paddingTop: 8,
    paddingBottom: 4,
    alignSelf: "flex-start",
  },
  note: {
    color: palette.secondary,
    marginHorizontal: layout.horizontal,
    marginTop: 20,
  },
  statGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 10,
    marginHorizontal: layout.horizontal,
    marginTop: 8,
  },
  stat: {
    flexBasis: "47%",
    flexGrow: 1,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.border,
    backgroundColor: palette.elevated,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  statLabel: {
    color: palette.secondary,
    fontSize: 10,
    letterSpacing: 1.4,
    fontWeight: "600",
  },
  statValue: {
    color: palette.text,
    fontSize: 22,
    fontWeight: "700",
    marginTop: 4,
  },
  card: {
    marginHorizontal: layout.horizontal,
    marginBottom: 12,
    borderRadius: layout.radius,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.border,
    backgroundColor: palette.elevated,
    padding: 16,
    gap: 8,
  },
  cardTitle: { color: palette.text, fontSize: 15, fontWeight: "600" },
  upgradeLine: { color: palette.accentSoft, fontSize: 12, marginTop: 2 },
  bar: {
    flexDirection: "row",
    height: 10,
    borderRadius: 5,
    overflow: "hidden",
    backgroundColor: palette.elevatedStrong,
    marginVertical: 4,
  },
  legendRow: { flexDirection: "row", alignItems: "center", gap: 10 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  legendLabel: { color: palette.text, flex: 1, fontSize: 14 },
  legendValue: {
    color: palette.secondary,
    fontSize: 14,
    fontVariant: ["tabular-nums"],
  },
  subtle: { color: palette.secondary, fontSize: 12 },
  actions: {
    marginHorizontal: layout.horizontal,
    gap: 10,
    marginBottom: 12,
  },
  action: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
    borderRadius: 999,
    paddingVertical: 13,
    backgroundColor: palette.elevatedStrong,
  },
  actionPrimary: { backgroundColor: "white" },
  actionText: { color: "white", fontWeight: "600", fontSize: 15 },
  error: {
    color: palette.danger,
    marginHorizontal: layout.horizontal,
    marginBottom: 12,
  },
  fileRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 6,
  },
  fileName: { color: palette.text, fontSize: 14, fontWeight: "500" },
  job: {
    marginHorizontal: layout.horizontal,
    marginBottom: 10,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.border,
    backgroundColor: palette.elevated,
    padding: 12,
    gap: 10,
  },
  progressTrack: {
    height: 4,
    borderRadius: 2,
    backgroundColor: palette.elevatedStrong,
    overflow: "hidden",
  },
  progressFill: { height: 4, borderRadius: 2, backgroundColor: ACCENT },
});
