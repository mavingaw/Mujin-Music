import {
  createImportJob,
  getImportJobs,
  getImportServices,
  importErrorMessage,
  parseImportUrl,
  PROVIDER_CAPABILITY_LABELS,
  type ImportService,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { ArrowLeft, Check, ChevronRight, X } from "lucide-react-native";
import { useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { PageTitle, Screen, SectionTitle } from "@/components/music-ui";
import { alpha, layout, palette } from "@/constants/colors";
import {
  importStatusLine,
  isJobRunning,
  SERVICE_COLORS,
  SERVICE_NAMES,
} from "@/lib/imports";
import { immediatePressFeedback } from "@/lib/press-feedback";

function ServiceRow({ service }: { service: ImportService }) {
  return (
    <View style={styles.serviceRow}>
      <View style={styles.serviceHeader}>
        <View
          style={[styles.dot, { backgroundColor: SERVICE_COLORS[service.id] }]}
        />
        <Text style={styles.serviceName}>{service.name}</Text>
        <Text
          style={[
            styles.serviceState,
            service.connection ? styles.connected : null,
          ]}
        >
          {service.connection
            ? (service.connection.account_label ?? "Connected")
            : service.id === "deezer" || service.id === "file"
              ? "No sign-in needed"
              : "Not connected"}
        </Text>
      </View>
      <View style={styles.chips}>
        {service.capabilities.map((grant) => (
          <View
            accessibilityLabel={`${PROVIDER_CAPABILITY_LABELS[grant.capability]} ${
              grant.available ? "available" : "not available"
            }: ${grant.detail}`}
            key={grant.capability}
            style={[styles.chip, grant.available && styles.chipOn]}
          >
            {grant.available ? (
              <Check color={palette.success} size={11} />
            ) : (
              <X color={palette.muted} size={11} />
            )}
            <Text
              style={[
                styles.chipText,
                grant.available ? styles.chipTextOn : styles.chipTextOff,
              ]}
            >
              {PROVIDER_CAPABILITY_LABELS[grant.capability]}
            </Text>
          </View>
        ))}
      </View>
    </View>
  );
}

export default function ImportsScreen() {
  const router = useRouter();
  const client = useQueryClient();
  const [link, setLink] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const parsed = parseImportUrl(link);
  const services = useQuery({
    queryKey: ["import-services"],
    queryFn: getImportServices,
  });
  const jobs = useQuery({
    queryKey: ["import-jobs"],
    queryFn: getImportJobs,
    refetchInterval: (query) =>
      query.state.data?.some(isJobRunning) ? 1_500 : false,
  });

  const start = async () => {
    if (!parsed || busy) return;
    setBusy(true);
    setMessage("");
    try {
      const job = await createImportJob({ url: link.trim() });
      setLink("");
      void client.invalidateQueries({ queryKey: ["import-jobs"] });
      router.push(`/imports/${job.id}`);
    } catch (error) {
      setMessage(importErrorMessage(error, "The import did not start."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <ScrollView
          contentContainerStyle={{
            paddingBottom: layout.miniPlayer + 40,
          }}
          keyboardShouldPersistTaps="handled"
        >
          <Pressable
            accessibilityLabel="Back"
            accessibilityRole="button"
            onPress={() => router.back()}
            style={styles.back}
          >
            <ArrowLeft color={palette.text} size={22} />
          </Pressable>
          <PageTitle subtitle="Matched to music you own. Nothing is streamed from other services.">
            Import playlists
          </PageTitle>
          <View style={styles.section}>
            <TextInput
              accessibilityLabel="Playlist link"
              autoCapitalize="none"
              autoCorrect={false}
              onChangeText={setLink}
              onSubmitEditing={() => void start()}
              placeholder="Paste a Spotify, Apple Music, YouTube Music or Deezer link"
              placeholderTextColor={palette.muted}
              style={styles.input}
              value={link}
            />
            <Text style={styles.hint}>
              {link.trim()
                ? parsed
                  ? `${SERVICE_NAMES[parsed.service]} ${parsed.kind.replace("_", " ")}`
                  : "Not a supported playlist link"
                : "Files and service sign-in: Settings › Connections in Mujin Music on the web or desktop."}
            </Text>
            <Pressable
              {...immediatePressFeedback}
              accessibilityRole="button"
              disabled={!parsed || busy}
              onPress={() => void start()}
              style={[styles.primary, (!parsed || busy) && styles.disabled]}
            >
              {busy ? (
                <ActivityIndicator color={palette.background} />
              ) : (
                <Text style={styles.primaryText}>Import</Text>
              )}
            </Pressable>
            {message ? <Text style={styles.error}>{message}</Text> : null}
          </View>

          {jobs.data?.length ? (
            <>
              <SectionTitle>Recent imports</SectionTitle>
              <View style={styles.section}>
                {jobs.data.map((job) => (
                  <Pressable
                    {...immediatePressFeedback}
                    accessibilityRole="button"
                    key={job.id}
                    onPress={() => router.push(`/imports/${job.id}`)}
                    style={styles.jobRow}
                  >
                    <View
                      style={[
                        styles.dot,
                        { backgroundColor: SERVICE_COLORS[job.service] },
                      ]}
                    />
                    <View style={{ flex: 1 }}>
                      <Text numberOfLines={1} style={styles.jobName}>
                        {job.name}
                      </Text>
                      <Text
                        numberOfLines={1}
                        style={[
                          styles.jobStatus,
                          job.status === "failed" && styles.errorText,
                        ]}
                      >
                        {importStatusLine(job)}
                      </Text>
                    </View>
                    {isJobRunning(job) ? (
                      <ActivityIndicator color={palette.secondary} />
                    ) : (
                      <ChevronRight color={palette.muted} size={18} />
                    )}
                  </Pressable>
                ))}
              </View>
            </>
          ) : null}

          <SectionTitle>What each service allows</SectionTitle>
          <View style={styles.section}>
            {services.isPending ? (
              <ActivityIndicator color={palette.text} />
            ) : (
              services.data?.map((service) => (
                <ServiceRow key={service.id} service={service} />
              ))
            )}
          </View>
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  back: {
    marginLeft: layout.horizontal - 6,
    marginTop: 8,
    height: 40,
    width: 40,
    alignItems: "center",
    justifyContent: "center",
  },
  section: { paddingHorizontal: layout.horizontal, gap: 10, marginBottom: 24 },
  input: {
    borderColor: palette.border,
    borderWidth: 1,
    borderRadius: 12,
    color: palette.text,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontSize: 15,
  },
  hint: { color: palette.secondary, fontSize: 12 },
  primary: {
    backgroundColor: palette.text,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
    height: 46,
  },
  primaryText: { color: palette.background, fontWeight: "800" },
  disabled: { opacity: 0.45 },
  error: { color: palette.danger, fontSize: 13 },
  errorText: { color: palette.danger },
  jobRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    borderBottomColor: palette.border,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  jobName: { color: palette.text, fontSize: 15, fontWeight: "600" },
  jobStatus: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  dot: { height: 8, width: 8, borderRadius: 4 },
  serviceRow: {
    backgroundColor: palette.elevated,
    borderColor: palette.border,
    borderWidth: 1,
    borderRadius: layout.radius,
    padding: 14,
    gap: 10,
  },
  serviceHeader: { flexDirection: "row", alignItems: "center", gap: 8 },
  serviceName: { color: palette.text, fontWeight: "700", fontSize: 15 },
  serviceState: {
    color: palette.muted,
    fontSize: 12,
    marginLeft: "auto",
    flexShrink: 1,
  },
  connected: { color: palette.success },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: palette.border,
    paddingHorizontal: 8,
    paddingVertical: 3,
  },
  chipOn: {
    borderColor: alpha(palette.success, 0.3),
    backgroundColor: alpha(palette.success, 0.08),
  },
  chipText: { fontSize: 11, fontWeight: "600" },
  chipTextOn: { color: palette.success },
  chipTextOff: { color: palette.muted, textDecorationLine: "line-through" },
});
