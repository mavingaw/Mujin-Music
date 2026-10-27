import {
  authorizeLastfm,
  connectListenbrainz,
  disconnectScrobbling,
  getScrobbling,
  importScrobbleHistory,
  plausibleListenbrainzToken,
  retryScrobbles,
  scrobbleServiceSummary,
  updateScrobbleSettings,
  type ScrobbleServiceStatus,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LinearGradient } from "expo-linear-gradient";
import * as WebBrowser from "expo-web-browser";
import { useState } from "react";
import {
  ActivityIndicator,
  Linking,
  Pressable,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
} from "react-native";

import { palette } from "@/constants/colors";
import { useSession } from "@/providers/session-provider";

const key = ["scrobbling"] as const;
const sky = "#38bdf8";
const tones = {
  ok: "#6ee7b7",
  warn: "#fcd34d",
  error: "#fda4af",
  idle: palette.secondary,
} as const;

const errorText = (error: unknown, fallback: string) =>
  (error as { response?: { data?: { message?: string } } })?.response?.data
    ?.message ?? fallback;

function Mark({ service }: { service: "lastfm" | "listenbrainz" }) {
  return (
    <LinearGradient
      colors={
        service === "lastfm" ? ["#ff3b30", "#b90000"] : ["#eb743b", "#353070"]
      }
      end={{ x: 1, y: 1 }}
      start={{ x: 0, y: 0 }}
      style={styles.mark}
    >
      <Text style={styles.markText}>{service === "lastfm" ? "fm" : "LB"}</Text>
    </LinearGradient>
  );
}

function Row({
  label,
  onChange,
  value,
}: {
  label: string;
  onChange: (value: boolean) => void;
  value: boolean;
}) {
  return (
    <View style={styles.toggle}>
      <Text style={styles.toggleLabel}>{label}</Text>
      <Switch
        accessibilityLabel={label}
        onValueChange={onChange}
        thumbColor="white"
        trackColor={{ false: "#3f3f46", true: sky }}
        value={value}
      />
    </View>
  );
}

function ServiceCard({ service }: { service: ScrobbleServiceStatus }) {
  const queryClient = useQueryClient();
  const { origin } = useSession();
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const summary = scrobbleServiceSummary(service);
  const connection = service.connection;

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setMessage(null);
    try {
      await action();
      await queryClient.invalidateQueries({ queryKey: key });
    } catch (cause) {
      setMessage(errorText(cause, "That did not work. Try again."));
    } finally {
      setBusy(false);
    }
  };

  const connectLastfm = () =>
    run(async () => {
      if (!origin) throw new Error("Not connected to a server.");
      const { authorizeUrl } = await authorizeLastfm(origin);
      // Last.fm returns to this server's page, which finishes connecting;
      // the status refreshes when the browser closes.
      await WebBrowser.openBrowserAsync(authorizeUrl).catch(() =>
        Linking.openURL(authorizeUrl),
      );
    });

  return (
    <View style={styles.card}>
      <View style={styles.header}>
        <Mark service={service.service} />
        <View style={{ flex: 1 }}>
          <Text style={styles.name}>{service.name}</Text>
          <Text
            numberOfLines={2}
            style={[styles.status, { color: tones[summary.tone] }]}
          >
            {connection ? `${connection.accountName} · ` : ""}
            {summary.text}
          </Text>
        </View>
        {busy ? <ActivityIndicator color="white" /> : null}
      </View>
      {connection?.lastError ? (
        <Text style={styles.note}>{connection.lastError}</Text>
      ) : null}
      {!connection || connection.status === "reauth_required" ? (
        service.service === "lastfm" ? (
          <Pressable
            accessibilityRole="button"
            disabled={!service.available || busy}
            onPress={() => void connectLastfm()}
            style={[styles.primary, !service.available && styles.disabled]}
          >
            <Text style={styles.primaryText}>
              {connection ? "Connect again" : "Connect Last.fm"}
            </Text>
          </Pressable>
        ) : (
          <View style={{ gap: 8 }}>
            <TextInput
              accessibilityLabel="ListenBrainz user token"
              autoCapitalize="none"
              autoCorrect={false}
              onChangeText={setToken}
              placeholder="User token from listenbrainz.org/settings"
              placeholderTextColor={palette.muted}
              style={styles.input}
              value={token}
            />
            <Pressable
              accessibilityRole="button"
              disabled={!plausibleListenbrainzToken(token) || busy}
              onPress={() =>
                void run(async () => {
                  await connectListenbrainz(token);
                  setToken("");
                })
              }
              style={[
                styles.primary,
                !plausibleListenbrainzToken(token) && styles.disabled,
              ]}
            >
              <Text style={styles.primaryText}>Connect ListenBrainz</Text>
            </Pressable>
          </View>
        )
      ) : (
        <>
          <Row
            label="Scrobble"
            onChange={(enabled) =>
              void run(() =>
                updateScrobbleSettings(service.service, { enabled }),
              )
            }
            value={connection.enabled}
          />
          <Row
            label="Now playing"
            onChange={(sendNowPlaying) =>
              void run(() =>
                updateScrobbleSettings(service.service, { sendNowPlaying }),
              )
            }
            value={connection.sendNowPlaying}
          />
          <Row
            label="Include Radio picks"
            onChange={(scrobbleRadio) =>
              void run(() =>
                updateScrobbleSettings(service.service, { scrobbleRadio }),
              )
            }
            value={connection.scrobbleRadio}
          />
          <Text style={styles.counts}>
            {service.queue.sent} sent · {service.queue.pending} waiting ·{" "}
            {service.queue.ignored + service.queue.failed} not taken
          </Text>
          {service.import ? (
            <Text style={styles.note}>
              {service.import.status === "running"
                ? `Importing… ${service.import.fetched} read, ${service.import.imported} added`
                : service.import.status === "done"
                  ? `History imported: ${service.import.imported} listens added`
                  : `Import stopped: ${service.import.error ?? ""}`}
            </Text>
          ) : null}
          <View style={styles.actions}>
            <Pressable
              accessibilityRole="button"
              disabled={busy || service.import?.status === "running"}
              onPress={() =>
                void run(() => importScrobbleHistory(service.service))
              }
              style={styles.secondary}
            >
              <Text style={styles.secondaryText}>Import history</Text>
            </Pressable>
            {service.queue.pending + service.queue.failed > 0 ? (
              <Pressable
                accessibilityRole="button"
                disabled={busy}
                onPress={() => void run(retryScrobbles)}
                style={styles.secondary}
              >
                <Text style={styles.secondaryText}>Retry now</Text>
              </Pressable>
            ) : null}
            <Pressable
              accessibilityRole="button"
              disabled={busy}
              onPress={() =>
                void run(() => disconnectScrobbling(service.service))
              }
              style={styles.secondary}
            >
              <Text style={[styles.secondaryText, { color: "#fda4af" }]}>
                Disconnect
              </Text>
            </Pressable>
          </View>
        </>
      )}
      {message ? <Text style={styles.error}>{message}</Text> : null}
    </View>
  );
}

/** Settings > Scrobbling: this profile's Last.fm and ListenBrainz. */
export function ScrobblingSettings() {
  const { claims } = useSession();
  const status = useQuery({
    queryKey: [...key, claims?.sub],
    queryFn: getScrobbling,
    refetchInterval: (query) =>
      query.state.data?.services.some(
        (service) =>
          service.import?.status === "running" || service.queue.pending > 0,
      )
        ? 3_000
        : 15_000,
  });
  if (status.isPending) return <ActivityIndicator color="white" />;
  if (!status.data)
    return <Text style={styles.body}>Scrobbling could not be loaded.</Text>;
  return (
    <View style={{ gap: 12 }}>
      <Text style={styles.heading}>Scrobbling</Text>
      <Text style={styles.body}>
        Send what this profile plays to Last.fm and ListenBrainz. Tracks longer
        than 30 seconds count after half their length or four minutes. Offline
        listens wait and are sent later.
      </Text>
      {status.data.services.map((service) => (
        <ServiceCard key={service.service} service={service} />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  heading: { color: "white", fontSize: 19, fontWeight: "800" },
  body: { color: palette.secondary, lineHeight: 21 },
  card: {
    padding: 16,
    borderRadius: 16,
    backgroundColor: palette.elevatedStrong,
    gap: 10,
  },
  header: { flexDirection: "row", alignItems: "center", gap: 12 },
  mark: {
    width: 42,
    height: 42,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  markText: { color: "white", fontWeight: "900", fontSize: 15 },
  name: { color: "white", fontSize: 15, fontWeight: "800" },
  status: { fontSize: 13, marginTop: 3 },
  note: { color: palette.secondary, fontSize: 13, lineHeight: 19 },
  error: { color: "#fda4af", fontSize: 13 },
  counts: { color: palette.secondary, fontSize: 13 },
  toggle: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  toggleLabel: { color: "white", fontSize: 15 },
  input: {
    height: 48,
    borderRadius: 10,
    backgroundColor: "#0b0b0d",
    borderWidth: 1,
    borderColor: palette.border,
    color: "white",
    paddingHorizontal: 14,
  },
  primary: {
    height: 46,
    borderRadius: 23,
    backgroundColor: "white",
    alignItems: "center",
    justifyContent: "center",
  },
  primaryText: { color: "black", fontWeight: "800" },
  disabled: { opacity: 0.4 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  secondary: {
    height: 36,
    borderRadius: 18,
    paddingHorizontal: 14,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    justifyContent: "center",
  },
  secondaryText: { color: "white", fontSize: 13, fontWeight: "700" },
});
