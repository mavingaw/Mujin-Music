import { Image } from "expo-image";
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useEffect, useRef, useState } from "react";
import { validPasswordLength } from "@parson/music-sdk";

import { LibraryList, SearchingIndicator } from "@/components/library-list";
import { palette } from "@/constants/colors";
import { useLibraryDiscovery } from "@/hooks/use-library-discovery";
import { SERVER_REPLACEMENT_WARNING } from "@/lib/discovery-manifest";
import { libraryRows } from "@/lib/known-libraries";
import { useSession } from "@/providers/session-provider";

export default function EntryScreen() {
  const session = useSession();
  const nearbyLibraries = useLibraryDiscovery();
  const [server, setServer] = useState(session.origin ?? "");
  const [serverEdited, setServerEdited] = useState(false);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [libraryPath, setLibraryPath] = useState("");
  const [showCustomFolder, setShowCustomFolder] = useState(false);
  const [passwordLoginOrigin, setPasswordLoginOrigin] = useState<string | null>(
    null,
  );
  const pairingStartedFor = useRef<string | null>(null);
  const [connectingOrigin, setConnectingOrigin] = useState<string | null>(null);
  const passwordLogin = passwordLoginOrigin === session.origin;
  const serverValue = serverEdited ? server : (session.origin ?? server);
  useEffect(() => {
    if (
      Platform.OS === "web" ||
      session.phase !== "login" ||
      passwordLogin ||
      !session.origin ||
      session.savedAccounts.length > 0 ||
      pairingStartedFor.current === session.origin
    )
      return;
    pairingStartedFor.current = session.origin;
    void session.startPairing();
  }, [passwordLogin, session]);
  if (session.phase === "ready" || session.phase === "offline") return null;

  const libraries = libraryRows({
    known: session.knownLibraries,
    nearby: nearbyLibraries.map((library) => ({
      instanceId: library.manifest.instanceId,
      origin: library.origin,
      name: library.manifest.name,
    })),
    currentInstanceId: null,
    accountInstanceIds: session.accountLibraryIds,
  });
  const openLibrary = (origin: string) => {
    setConnectingOrigin(origin);
    void session.switchLibrary(origin).finally(() => setConnectingOrigin(null));
  };

  const busy =
    session.phase === "loading" ||
    session.phase === "connecting" ||
    session.phase === "indexing";
  const invalidAccount = !username.trim() || !validPasswordLength(password);
  const libraryTarget =
    libraryPath.trim() ||
    session.setupStatus?.suggested_library_path?.trim() ||
    "";
  const noPlayableMusic = Boolean(
    session.error?.toLowerCase().includes("no supported audio") ||
    session.error?.toLowerCase().includes("no playable"),
  );
  const connectionFailed =
    Platform.OS === "android" &&
    session.phase === "discovering" &&
    Boolean(session.error) &&
    !session.serverReplacementPending;
  const hostSetupRequired =
    Platform.OS === "android" &&
    session.phase === "setup" &&
    Boolean(session.setupStatus?.setup_required);
  const heading =
    session.phase === "login"
      ? passwordLogin
        ? "Welcome back"
        : session.savedAccounts.length && !session.pairing
          ? "Choose an account"
          : "Pair your phone"
      : session.phase === "indexing"
        ? "Preparing your library"
        : session.phase === "setup"
          ? hostSetupRequired
            ? "Finish setup on the Mujin Music host"
            : session.setupStatus?.account_setup_required
              ? "Create your account"
              : "Choose your music folder"
          : "Finding your library";
  const detail =
    session.phase === "indexing"
      ? "Mujin Music is indexing your music. This screen will update automatically."
      : session.phase === "setup"
        ? hostSetupRequired
          ? "Create the administrator account and choose the music folder on the computer running Mujin Music."
          : (session.setupStatus?.message ??
            "Finish setting up this Mujin Music library.")
        : session.phase === "login"
          ? passwordLogin
            ? (session.libraryName ?? "Sign in to your Mujin Music library")
            : session.savedAccounts.length && !session.pairing
              ? `Continue on ${session.libraryName ?? "your Mujin Music library"}.`
              : `Connect to ${session.libraryName ?? "this Mujin Music library"} without typing your account password.`
          : "";

  return (
    <KeyboardAvoidingView
      behavior={Platform.OS === "ios" ? "padding" : "height"}
      style={styles.page}
    >
      <ScrollView
        contentContainerStyle={styles.scrollContent}
        keyboardShouldPersistTaps="handled"
        style={styles.scroll}
      >
        <View style={styles.content}>
          <Image
            source={require("@/assets/images/mujin-symbol.png")}
            style={styles.logo}
          />
          <Text style={styles.heading}>{heading}</Text>
          {detail ? <Text style={styles.detail}>{detail}</Text> : null}
          {busy && (
            <ActivityIndicator
              color="white"
              size="small"
              style={styles.spinner}
            />
          )}
          {session.serverReplacementPending ? (
            <View accessibilityRole="alert" style={styles.failureCard}>
              <Text style={styles.failureTitle}>Library identity changed</Text>
              <Text style={styles.failureBody}>
                {SERVER_REPLACEMENT_WARNING}
              </Text>
              <View style={styles.replacementActions}>
                <Pressable
                  accessibilityRole="button"
                  disabled={busy}
                  style={[styles.primary, styles.replacementPrimary]}
                  onPress={() => void session.confirmServerReplacement()}
                >
                  <Text style={styles.primaryText}>Trust this library</Text>
                </Pressable>
                <Pressable
                  accessibilityRole="button"
                  disabled={busy}
                  onPress={session.cancelServerReplacement}
                >
                  <Text style={styles.link}>Keep saved library</Text>
                </Pressable>
              </View>
            </View>
          ) : connectionFailed ? (
            <View accessibilityRole="alert" style={styles.failureCard}>
              <Text style={styles.failureTitle}>Android can’t connect</Text>
              <Text style={styles.failureBody}>
                Make sure the Mujin Music host is running and this phone is on
                the same Wi‑Fi network. Then check the address and try again.
              </Text>
              <Text selectable style={styles.failureDetail}>
                {session.error}
              </Text>
            </View>
          ) : noPlayableMusic ? (
            <View accessibilityRole="alert" style={styles.failureCard}>
              <Text style={styles.failureTitle}>No playable music found</Text>
              <Text style={styles.failureBody}>
                This folder doesn’t contain audio that Mujin Music can play.
                Choose a different folder with supported music files.
              </Text>
            </View>
          ) : null}
          {session.phase === "setup" ? (
            <View style={styles.form}>
              {hostSetupRequired ? null : session.setupStatus
                  ?.account_setup_required ? (
                <>
                  <TextInput
                    accessibilityLabel="Admin username"
                    autoCapitalize="none"
                    autoCorrect={false}
                    placeholder="Admin username"
                    placeholderTextColor={palette.muted}
                    style={styles.input}
                    value={username}
                    onChangeText={setUsername}
                  />
                  <TextInput
                    accessibilityLabel="Password"
                    placeholder="Password"
                    placeholderTextColor={palette.muted}
                    secureTextEntry
                    style={styles.input}
                    value={password}
                    onChangeText={setPassword}
                  />
                  <Pressable
                    accessibilityRole="button"
                    disabled={busy || invalidAccount}
                    style={[
                      styles.primary,
                      (busy || invalidAccount) && styles.disabledPrimary,
                    ]}
                    onPress={() =>
                      void session.setupAccount(username, password)
                    }
                  >
                    <Text style={styles.primaryText}>Create account</Text>
                  </Pressable>
                </>
              ) : (
                <>
                  <View style={styles.folderCard}>
                    <Text style={styles.folderLabel}>Music folder</Text>
                    <Text style={styles.folderPath}>{libraryTarget}</Text>
                  </View>
                  <Pressable
                    accessibilityRole="button"
                    disabled={busy || !libraryTarget}
                    style={[
                      styles.primary,
                      (busy || !libraryTarget) && styles.disabledPrimary,
                    ]}
                    onPress={() => void session.setupLibrary(libraryTarget)}
                  >
                    <Text style={styles.primaryText}>Use this folder</Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    disabled={busy}
                    onPress={() => setShowCustomFolder((shown) => !shown)}
                  >
                    <Text style={styles.link}>
                      {showCustomFolder
                        ? "Hide different folder"
                        : "Choose a different folder"}
                    </Text>
                  </Pressable>
                  {showCustomFolder ? (
                    <TextInput
                      accessibilityLabel="Music folder path"
                      autoCapitalize="none"
                      autoCorrect={false}
                      placeholder="Music folder path"
                      placeholderTextColor={palette.muted}
                      style={styles.input}
                      value={libraryPath}
                      editable={!busy}
                      onChangeText={setLibraryPath}
                    />
                  ) : null}
                </>
              )}
            </View>
          ) : session.phase === "login" && !passwordLogin ? (
            <View style={styles.form}>
              {session.savedAccounts.length && !session.pairing ? (
                <>
                  <View style={styles.nearbySection}>
                    {session.savedAccounts.map((account) => (
                      <Pressable
                        accessibilityHint="Switches to this saved Mujin Music account"
                        accessibilityRole="button"
                        disabled={busy}
                        key={account.key}
                        style={({ pressed }) => [
                          styles.libraryCard,
                          pressed && styles.libraryCardPressed,
                          busy && styles.disabledPrimary,
                        ]}
                        onPress={() => void session.switchAccount(account.key)}
                      >
                        <Text style={styles.libraryName}>
                          {account.username}
                        </Text>
                        <Text style={styles.libraryAddress}>
                          {session.libraryName}
                        </Text>
                      </Pressable>
                    ))}
                  </View>
                  {session.error ? (
                    <Text accessibilityRole="alert" style={styles.error}>
                      {session.error}
                    </Text>
                  ) : null}
                  <Pressable
                    accessibilityRole="button"
                    style={styles.primary}
                    onPress={() => {
                      pairingStartedFor.current = session.origin;
                      void session.startPairing();
                    }}
                  >
                    <Text style={styles.primaryText}>Pair another account</Text>
                  </Pressable>
                </>
              ) : session.pairing ? (
                <View style={styles.pairingCard}>
                  <Text style={styles.pairingLabel}>PAIRING CODE</Text>
                  <Text
                    accessibilityLabel={`Pairing code ${session.pairing.code
                      .split("")
                      .join(" ")}`}
                    selectable
                    style={styles.pairingCode}
                  >
                    {session.pairing.code.replace(/^(\d{3})(\d{3})$/, "$1 $2")}
                  </Text>
                  <Text style={styles.pairingHelp}>
                    In the Mujin Music web app, open Settings → Connections,
                    enter this code, and approve this phone.
                  </Text>
                  <ActivityIndicator
                    color="white"
                    size="small"
                    style={styles.pairingSpinner}
                  />
                </View>
              ) : (
                <View style={styles.pairingLoading}>
                  <ActivityIndicator color="white" size="small" />
                  <Text style={styles.discoveryStatus}>
                    Creating a secure pairing code…
                  </Text>
                </View>
              )}
              {session.error &&
              (!session.savedAccounts.length || session.pairing) ? (
                <>
                  <Text accessibilityRole="alert" style={styles.error}>
                    {session.error}
                  </Text>
                  <Pressable
                    accessibilityRole="button"
                    style={styles.primary}
                    onPress={() => {
                      pairingStartedFor.current = session.origin;
                      void session.startPairing();
                    }}
                  >
                    <Text style={styles.primaryText}>Request a new code</Text>
                  </Pressable>
                </>
              ) : null}
              <Pressable
                accessibilityRole="button"
                onPress={() => {
                  session.cancelPairing();
                  setPasswordLoginOrigin(session.origin);
                }}
              >
                <Text style={styles.link}>Sign in with password instead</Text>
              </Pressable>
              <Pressable
                accessibilityRole="button"
                onPress={() => void session.changeServer()}
              >
                <Text style={styles.secondaryLink}>Choose another library</Text>
              </Pressable>
            </View>
          ) : session.phase === "login" ? (
            <View style={styles.form}>
              <TextInput
                accessibilityLabel="Username"
                autoCapitalize="none"
                autoCorrect={false}
                placeholder="Username"
                placeholderTextColor={palette.muted}
                style={styles.input}
                value={username}
                onChangeText={setUsername}
              />
              <TextInput
                accessibilityLabel="Password"
                placeholder="Password"
                placeholderTextColor={palette.muted}
                secureTextEntry
                style={styles.input}
                value={password}
                onChangeText={setPassword}
                onSubmitEditing={() => void session.login(username, password)}
              />
              <Pressable
                accessibilityRole="button"
                disabled={busy || !username.trim() || !password}
                style={[
                  styles.primary,
                  (busy || !username.trim() || !password) &&
                    styles.disabledPrimary,
                ]}
                onPress={() => void session.login(username, password)}
              >
                <Text style={styles.primaryText}>Sign in</Text>
              </Pressable>
              <Pressable
                accessibilityRole="button"
                onPress={() => {
                  setPasswordLoginOrigin(null);
                  pairingStartedFor.current = null;
                }}
              >
                <Text style={styles.link}>Pair this phone instead</Text>
              </Pressable>
            </View>
          ) : session.phase === "discovering" ||
            session.phase === "connecting" ||
            session.phase === "loading" ? (
            <View style={styles.form}>
              {libraries.length ? (
                <View style={styles.nearbySection}>
                  <Text style={styles.sectionLabel}>
                    {libraries.some((library) => library.known)
                      ? "Your libraries"
                      : "Nearby"}
                  </Text>
                  <LibraryList
                    busy={busy}
                    connectingOrigin={connectingOrigin}
                    onForget={(row) =>
                      void session.forgetLibrary(row.instanceId)
                    }
                    onSelect={(row) => openLibrary(row.origin)}
                    rows={libraries}
                  />
                </View>
              ) : null}
              {session.phase === "discovering" ? (
                <SearchingIndicator
                  label={
                    libraries.some((library) => library.nearby)
                      ? "Still looking for more…"
                      : "Looking for Mujin Music libraries on this Wi‑Fi…"
                  }
                />
              ) : null}
              <Text style={styles.sectionLabel}>Manual address</Text>
              <TextInput
                accessibilityLabel="Server address"
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
                returnKeyType="go"
                placeholder="192.168.1.10:1993"
                placeholderTextColor={palette.muted}
                style={styles.input}
                value={serverValue}
                onChangeText={(value) => {
                  setServerEdited(true);
                  setServer(value);
                }}
                onSubmitEditing={() => void session.connect(serverValue)}
              />
              <Pressable
                accessibilityRole="button"
                disabled={busy || !serverValue.trim()}
                style={[styles.primary, busy && styles.disabledPrimary]}
                onPress={() => void session.connect(serverValue)}
              >
                <Text style={styles.primaryText}>
                  {session.phase === "connecting" ? "Connecting…" : "Connect"}
                </Text>
              </Pressable>
            </View>
          ) : null}
          {session.error &&
          !connectionFailed &&
          !noPlayableMusic &&
          !session.serverReplacementPending &&
          ((session.phase === "login" && passwordLogin) ||
            session.phase === "setup" ||
            session.phase === "discovering") ? (
            <Text accessibilityRole="alert" style={styles.error}>
              {session.error}
            </Text>
          ) : null}
        </View>
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  page: {
    flex: 1,
    backgroundColor: "black",
  },
  scroll: { width: "100%" },
  scrollContent: {
    flexGrow: 1,
    justifyContent: "center",
    paddingHorizontal: 28,
    paddingVertical: 28,
  },
  content: {
    alignItems: "center",
    width: "100%",
    maxWidth: 440,
    alignSelf: "center",
  },
  logo: { width: 72, height: 72, borderRadius: 18, marginBottom: 26 },
  heading: {
    color: "white",
    fontSize: 28,
    fontWeight: "800",
    letterSpacing: -0.6,
    textAlign: "center",
  },
  detail: {
    color: palette.secondary,
    textAlign: "center",
    fontSize: 15,
    lineHeight: 22,
    marginTop: 10,
    maxWidth: 330,
  },
  spinner: { marginTop: 25 },
  form: { width: "100%", gap: 12, marginTop: 28 },
  input: {
    height: 52,
    borderRadius: 12,
    backgroundColor: palette.elevatedStrong,
    borderWidth: 1,
    borderColor: palette.border,
    color: "white",
    paddingHorizontal: 16,
    fontSize: 16,
  },
  primary: {
    height: 52,
    backgroundColor: "white",
    borderRadius: 26,
    alignItems: "center",
    justifyContent: "center",
  },
  primaryText: { color: "black", fontSize: 16, fontWeight: "800" },
  disabledPrimary: { opacity: 0.62 },
  link: { color: "white", textAlign: "center", fontWeight: "700", padding: 12 },
  secondaryLink: {
    color: palette.secondary,
    textAlign: "center",
    fontWeight: "600",
    padding: 8,
  },
  pairingCard: {
    alignItems: "center",
    borderRadius: 16,
    borderWidth: 1,
    borderColor: palette.border,
    backgroundColor: palette.elevated,
    paddingHorizontal: 20,
    paddingVertical: 22,
  },
  pairingLabel: {
    color: palette.secondary,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.4,
  },
  pairingCode: {
    color: "white",
    fontSize: 38,
    fontWeight: "900",
    letterSpacing: 4,
    marginTop: 9,
    fontVariant: ["tabular-nums"],
  },
  pairingHelp: {
    color: palette.secondary,
    lineHeight: 20,
    textAlign: "center",
    marginTop: 12,
    maxWidth: 310,
  },
  pairingSpinner: { marginTop: 18 },
  pairingLoading: {
    minHeight: 128,
    alignItems: "center",
    justifyContent: "center",
    gap: 14,
  },
  nearbySection: { gap: 10, marginBottom: 8 },
  sectionLabel: {
    color: palette.secondary,
    fontSize: 13,
    fontWeight: "800",
    letterSpacing: 0.5,
    textTransform: "uppercase",
  },
  discoveryStatus: {
    color: palette.muted,
    fontSize: 14,
    lineHeight: 20,
    marginBottom: 8,
  },
  libraryCard: {
    borderRadius: 12,
    borderWidth: 1,
    borderColor: palette.border,
    backgroundColor: palette.elevatedStrong,
    paddingHorizontal: 16,
    paddingVertical: 13,
  },
  libraryCardPressed: { opacity: 0.78 },
  libraryName: { color: "white", fontSize: 16, fontWeight: "800" },
  libraryAddress: { color: palette.muted, fontSize: 13, marginTop: 4 },
  folderCard: {
    borderRadius: 12,
    borderWidth: 1,
    borderColor: palette.border,
    backgroundColor: palette.elevatedStrong,
    paddingHorizontal: 16,
    paddingVertical: 14,
  },
  folderLabel: { color: "white", fontSize: 14, fontWeight: "700" },
  folderPath: { color: palette.muted, fontSize: 14, marginTop: 4 },
  error: { color: "#fb7185", textAlign: "center", marginTop: 16 },
  failureCard: {
    width: "100%",
    marginTop: 22,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: "#713f12",
    backgroundColor: "#1c1408",
    padding: 18,
  },
  failureTitle: { color: "white", fontSize: 19, fontWeight: "800" },
  failureBody: {
    color: palette.secondary,
    fontSize: 14,
    lineHeight: 21,
    marginTop: 7,
  },
  failureDetail: {
    color: palette.muted,
    fontSize: 12,
    lineHeight: 18,
    marginTop: 10,
  },
  replacementActions: { gap: 2, marginTop: 16 },
  replacementPrimary: { width: "100%" },
});
