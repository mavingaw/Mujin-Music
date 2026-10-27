import {
  accountProof,
  type AccountProofTone,
  checkStreamingConnection,
  formatLastSync,
  getSourcePreference,
  getStreamingProviders,
  SOURCE_PREFERENCE_LABELS,
  SOURCE_PREFERENCES,
  setSourcePreference,
  pollTidalSignIn,
  saveStreamingCredentials,
  setStreamingQuality,
  signInToQobuz,
  signOutOfStreamingProvider,
  startTidalSignIn,
  syncStreamingLibrary,
  type StreamingPendingSignIn,
  type StreamingProviderStatus,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  Linking,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";

import { SourceTag } from "@/components/streaming-ui";
import { palette } from "@/constants/colors";
import {
  codeSecondsLeft,
  keysNoticeText,
  needsOwnCredentials,
  signInThroughText,
  subscriberSetupText,
  serviceAction,
  tidalPhoneSignIn,
} from "@/lib/streaming";

const errorText = (error: unknown, fallback: string) =>
  (error as { response?: { data?: { message?: string } } })?.response?.data
    ?.message ?? fallback;

/** Settings > Services: connect TIDAL and Qobuz with your own credentials. */
export function StreamingServicesSettings() {
  const providers = useQuery({
    queryKey: ["streaming", "providers"],
    queryFn: getStreamingProviders,
  });
  const [open, setOpen] = useState<string | null>(null);
  if (providers.isPending) return <ActivityIndicator color="white" />;
  if (!providers.data)
    return (
      <Pressable
        accessibilityRole="button"
        onPress={() => void providers.refetch()}
      >
        <Text style={styles.body}>
          Services could not be loaded · Tap to retry
        </Text>
      </Pressable>
    );
  return (
    <View style={{ gap: 14 }}>
      <Text style={styles.heading}>Services</Text>
      <Text style={styles.body}>
        Connect TIDAL and Qobuz to search and play their catalogs next to your
        own files.
      </Text>
      {providers.data.sandbox ? (
        <Text style={styles.sandbox}>
          Sandbox mode: recorded TIDAL and Qobuz responses, and 30-second
          synthesized tones rendered in each announced format (24/192 FLAC,
          TIDAL Max DASH, CD, AAC). Sign-in goes through a simulated Mujin Music
          key service, as it would for a subscriber; any email and password
          work. No real account is used.
        </Text>
      ) : null}
      {providers.data.providers.map((provider) => (
        <View key={provider.id} style={styles.card}>
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ expanded: open === provider.id }}
            onPress={() => setOpen(open === provider.id ? null : provider.id)}
            style={styles.cardHeader}
          >
            {/* Name above status, so the status gets the row's full width
                on narrow phones instead of a squeezed middle column. */}
            <View style={styles.cardTitle}>
              <SourceTag large provider={provider.id} />
              <Text numberOfLines={2} style={styles.status}>
                {provider.status === "signed_in"
                  ? `${provider.account_name ?? "Signed in"} · ${provider.library.albums} album${provider.library.albums === 1 ? "" : "s"}`
                  : provider.status === "expired"
                    ? "Session expired"
                    : needsOwnCredentials(provider)
                      ? "Not configured"
                      : "Not signed in"}
              </Text>
            </View>
            <Text style={styles.link}>{serviceAction(provider)}</Text>
          </Pressable>
          {provider.status === "signed_in" ? (
            <AccountProofLine provider={provider} />
          ) : null}
          {open === provider.id ? <ServicePanel provider={provider} /> : null}
        </View>
      ))}
      <SourcePreferencePicker />
      <Text style={styles.fine}>
        {keysNoticeText(providers.data.providers)}
      </Text>
      <Text style={styles.fine}>
        Built and tested against recorded TIDAL and Qobuz responses and media
        rendered to match them. It has not yet been run against a live TIDAL or
        Qobuz account, so full-length hi-res from the real services is unproven
        until your own Check connection passes; only then does an account show
        Verified. Subscription-only sign-in through a key service was tested
        with the key service&apos;s own code against recorded replies; no key
        service with real TIDAL or Qobuz licences exists yet. See How to get
        credentials in the web app&apos;s Settings, Services.
      </Text>
    </View>
  );
}

const proofColors: Record<AccountProofTone, string> = {
  verified: palette.success,
  partial: "#fde68a",
  failed: "#fda4af",
  unproven: palette.muted,
};

/** Whether this account itself has streamed a whole lossless track. */
function AccountProofLine({ provider }: { provider: StreamingProviderStatus }) {
  const proof = accountProof(provider);
  if (!proof) return null;
  return (
    <View accessible style={styles.proof}>
      <Text style={[styles.proofLabel, { color: proofColors[proof.tone] }]}>
        {proof.tone === "verified" ? "✓ " : ""}
        {proof.label}
      </Text>
      <Text style={styles.fine}>{proof.detail}</Text>
    </View>
  );
}

/** Which copy plays when an album is in your files and on a service. */
function SourcePreferencePicker() {
  const client = useQueryClient();
  const preference = useQuery({
    queryKey: ["streaming", "source-preference"],
    queryFn: getSourcePreference,
  });
  const save = useMutation({
    mutationFn: setSourcePreference,
    onSuccess: (source) =>
      client.setQueryData(["streaming", "source-preference"], source),
  });
  const current = preference.data ?? "highest_quality";
  return (
    <View style={{ gap: 8 }}>
      <Text style={styles.subheading}>
        When an album is in more than one place
      </Text>
      {SOURCE_PREFERENCES.map((source) => (
        <Pressable
          accessibilityRole="radio"
          accessibilityState={{ selected: current === source }}
          disabled={save.isPending}
          key={source}
          onPress={() => save.mutate(source)}
          style={[styles.option, current === source && styles.selected]}
        >
          <Text style={styles.optionText}>
            {current === source ? "✓  " : ""}
            {SOURCE_PREFERENCE_LABELS[source]}
          </Text>
        </Pressable>
      ))}
      {save.isError ? (
        <Text style={styles.warning}>Could not save. Try again.</Text>
      ) : null}
    </View>
  );
}

function ServicePanel({ provider }: { provider: StreamingProviderStatus }) {
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState(needsOwnCredentials(provider));
  // Without server credentials, explain the subscriber path before showing
  // an API credentials form.
  const [chooseOwn, setChooseOwn] = useState(
    provider.sandbox || !needsOwnCredentials(provider),
  );
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState<StreamingPendingSignIn | null>(
    provider.pending,
  );
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState(provider.status_message ?? "");
  const [check, setCheck] = useState("");
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: ["streaming"] });
  const run = async (task: () => Promise<unknown>, fallback: string) => {
    setBusy(true);
    setMessage("");
    try {
      await task();
      await refresh();
    } catch (error) {
      setMessage(errorText(error, fallback));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (!pending) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = (delay: number) => {
      timer = setTimeout(async () => {
        try {
          const result = await pollTidalSignIn();
          if (cancelled) return;
          if (result.status === "pending") return poll(result.interval * 1000);
          setPending(null);
          if (result.status !== "signed_in")
            setMessage(
              result.status === "denied"
                ? "Sign-in was declined."
                : "The code expired.",
            );
          await refresh();
        } catch (error) {
          if (!cancelled) {
            setPending(null);
            setMessage(errorText(error, "Sign-in failed."));
          }
        }
      }, delay);
    };
    poll(pending.interval * 1000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending]);

  const labels =
    provider.id === "tidal"
      ? { id: "Client ID", secret: "Client secret (optional)" }
      : { id: "App ID", secret: "App secret" };

  return (
    <View style={styles.panel}>
      {editing && !chooseOwn ? (
        <>
          <Text style={styles.body}>
            {subscriberSetupText(provider.id, provider.name)}
          </Text>
          <Pressable
            accessibilityRole="button"
            onPress={() => setChooseOwn(true)}
            style={styles.primary}
          >
            <Text style={styles.primaryText}>
              I have my own {labels.id.toLowerCase()}
            </Text>
          </Pressable>
        </>
      ) : editing ? (
        <>
          <TextInput
            accessibilityLabel={labels.id}
            autoCapitalize="none"
            autoCorrect={false}
            onChangeText={setClientId}
            placeholder={provider.credentials.client_id_hint ?? labels.id}
            placeholderTextColor={palette.muted}
            style={styles.input}
            value={clientId}
          />
          <TextInput
            accessibilityLabel={labels.secret}
            autoCapitalize="none"
            autoCorrect={false}
            onChangeText={setSecret}
            placeholder={labels.secret}
            placeholderTextColor={palette.muted}
            secureTextEntry
            style={styles.input}
            value={secret}
          />
          <Pressable
            accessibilityRole="button"
            disabled={busy || !clientId.trim()}
            onPress={() =>
              void run(async () => {
                await saveStreamingCredentials(provider.id, {
                  client_id: clientId.trim(),
                  client_secret: secret.trim() || null,
                });
                setEditing(false);
              }, "Could not save credentials.")
            }
            style={[
              styles.primary,
              (busy || !clientId.trim()) && styles.disabled,
            ]}
          >
            <Text style={styles.primaryText}>Save credentials</Text>
          </Pressable>
          {provider.server_credentials_available ? (
            <Pressable
              accessibilityRole="button"
              disabled={busy}
              onPress={() =>
                void run(async () => {
                  await saveStreamingCredentials(provider.id, {
                    use_server_credentials: true,
                  });
                  setEditing(false);
                }, "Could not use the server's credentials.")
              }
            >
              <Text style={styles.link}>
                Use this server&apos;s credentials
              </Text>
            </Pressable>
          ) : null}
        </>
      ) : provider.status === "signed_in" ? (
        <>
          <Text style={styles.body}>
            {provider.library.albums}{" "}
            {provider.library.albums === 1 ? "album" : "albums"} ·{" "}
            {provider.library.tracks}{" "}
            {provider.library.tracks === 1 ? "track" : "tracks"} in library ·
            last sync {formatLastSync(provider.last_library_sync)}
          </Text>
          <Text style={styles.subheading}>Streaming quality</Text>
          {provider.quality_options.map((option) => (
            <Pressable
              accessibilityRole="radio"
              accessibilityState={{ selected: provider.quality === option.id }}
              disabled={busy}
              key={option.id}
              onPress={() =>
                void run(
                  () => setStreamingQuality(provider.id, option.id),
                  "Could not change quality.",
                )
              }
              style={[
                styles.option,
                provider.quality === option.id && styles.selected,
              ]}
            >
              <Text style={styles.optionText}>
                {provider.quality === option.id ? "✓  " : ""}
                {option.label}
              </Text>
            </Pressable>
          ))}
          <View style={styles.row}>
            <Pressable
              accessibilityRole="button"
              disabled={busy}
              onPress={() =>
                void run(
                  () => syncStreamingLibrary(provider.id),
                  "Could not sync.",
                )
              }
              style={styles.secondary}
            >
              <Text style={styles.secondaryText}>Sync library now</Text>
            </Pressable>
            <Pressable
              accessibilityRole="button"
              disabled={busy}
              onPress={() =>
                void run(
                  () => signOutOfStreamingProvider(provider.id),
                  "Could not sign out.",
                )
              }
              style={styles.secondary}
            >
              <Text style={styles.secondaryText}>Log out</Text>
            </Pressable>
          </View>
          <Pressable
            accessibilityRole="button"
            disabled={busy}
            onPress={() =>
              void run(async () => {
                const result = await checkStreamingConnection(provider.id);
                setCheck(
                  result.steps
                    .map((step) => `${step.ok ? "✓" : "✕"} ${step.detail}`)
                    .join("\n"),
                );
              }, "Could not run the check.")
            }
            style={styles.secondary}
          >
            <Text style={styles.secondaryText}>Check connection</Text>
          </Pressable>
          {check ? (
            <Text accessibilityLiveRegion="polite" style={styles.body}>
              {check}
            </Text>
          ) : null}
        </>
      ) : provider.id === "tidal" ? (
        pending ? (
          <View style={{ gap: 10, alignItems: "center" }}>
            <Text style={styles.body}>Approve this code on TIDAL:</Text>
            <Text accessibilityLiveRegion="polite" style={styles.code}>
              {pending.user_code}
            </Text>
            <Pressable
              accessibilityRole="link"
              onPress={() =>
                void Linking.openURL(
                  pending.verification_uri_complete ?? pending.verification_uri,
                )
              }
            >
              <Text style={styles.link}>Open TIDAL to approve</Text>
            </Pressable>
            <Text style={styles.fine}>
              Waiting… {Math.ceil(codeSecondsLeft(pending.expires_at) / 60)} min
              left
            </Text>
          </View>
        ) : tidalPhoneSignIn(provider).canStart ? (
          <Pressable
            accessibilityRole="button"
            disabled={busy}
            onPress={() =>
              void run(
                async () => setPending(await startTidalSignIn()),
                "Could not start sign-in.",
              )
            }
            style={[styles.primary, busy && styles.disabled]}
          >
            <Text style={styles.primaryText}>Sign in with TIDAL</Text>
          </Pressable>
        ) : (
          <Text style={styles.body}>{tidalPhoneSignIn(provider).note}</Text>
        )
      ) : (
        <>
          <TextInput
            accessibilityLabel="Qobuz email or username"
            autoCapitalize="none"
            autoComplete="username"
            onChangeText={setUsername}
            placeholder="Email or username"
            placeholderTextColor={palette.muted}
            style={styles.input}
            value={username}
          />
          <TextInput
            accessibilityLabel="Qobuz password"
            autoComplete="password"
            onChangeText={setPassword}
            placeholder="Password"
            placeholderTextColor={palette.muted}
            secureTextEntry
            style={styles.input}
            value={password}
          />
          <Text style={styles.fine}>
            Your password is sent to Qobuz once and never stored.
          </Text>
          <Pressable
            accessibilityRole="button"
            disabled={busy || !username.trim() || !password}
            onPress={() =>
              void run(async () => {
                await signInToQobuz(username.trim(), password);
                setPassword("");
              }, "Could not sign in to Qobuz.")
            }
            style={[
              styles.primary,
              (busy || !username.trim() || !password) && styles.disabled,
            ]}
          >
            <Text style={styles.primaryText}>Sign in</Text>
          </Pressable>
        </>
      )}
      {!editing ? (
        <Pressable accessibilityRole="button" onPress={() => setEditing(true)}>
          <Text style={styles.fine}>{signInThroughText(provider)}</Text>
        </Pressable>
      ) : null}
      {message ? <Text style={styles.warning}>{message}</Text> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  heading: { color: "white", fontSize: 19, fontWeight: "800" },
  subheading: { color: "white", fontWeight: "800", marginTop: 6 },
  body: { color: palette.secondary, lineHeight: 21 },
  fine: { color: palette.muted, fontSize: 12, lineHeight: 18 },
  proof: { gap: 2, paddingHorizontal: 16, paddingBottom: 12 },
  proofLabel: { fontSize: 13, fontWeight: "600", letterSpacing: 0.2 },
  sandbox: {
    color: "#fde68a",
    backgroundColor: "rgba(252,211,77,0.08)",
    borderRadius: 10,
    padding: 12,
    lineHeight: 19,
    overflow: "hidden",
  },
  warning: { color: "#fde68a" },
  card: {
    borderRadius: 14,
    backgroundColor: palette.elevatedStrong,
    overflow: "hidden",
  },
  cardHeader: {
    minHeight: 64,
    paddingHorizontal: 16,
    paddingVertical: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  cardTitle: { flex: 1, minWidth: 0, gap: 4 },
  status: { color: palette.secondary, fontSize: 13, lineHeight: 18 },
  link: { color: "#7dd3fc", fontWeight: "700" },
  panel: { padding: 16, paddingTop: 4, gap: 10 },
  input: {
    height: 48,
    borderRadius: 10,
    backgroundColor: palette.elevated,
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
  disabled: { opacity: 0.45 },
  option: {
    minHeight: 46,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: palette.border,
    paddingHorizontal: 14,
    justifyContent: "center",
  },
  selected: { borderColor: "white", backgroundColor: "#19191d" },
  optionText: { color: "white", fontWeight: "600" },
  row: { flexDirection: "row", gap: 10, marginTop: 4 },
  secondary: {
    flex: 1,
    height: 44,
    borderRadius: 22,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  secondaryText: { color: "white", fontWeight: "700" },
  code: {
    color: "white",
    fontSize: 34,
    fontWeight: "800",
    letterSpacing: 8,
    fontVariant: ["tabular-nums"],
  },
});
