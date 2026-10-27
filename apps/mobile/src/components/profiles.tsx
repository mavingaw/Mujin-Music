import {
  createProfile,
  getProfiles,
  profileGradient,
  profileInitials,
  switcherProfiles,
  validProfilePin,
  type Profile,
  type ProfileColor,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LinearGradient } from "expo-linear-gradient";
import { Check, Lock, Plus } from "lucide-react-native";
import { useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";

import { palette } from "@/constants/colors";
import { useSession } from "@/providers/session-provider";

const plays = new Intl.NumberFormat("en");

export function ProfileAvatar({
  color,
  name,
  size = 44,
}: {
  color: string;
  name: string;
  size?: number;
}) {
  const [from, to] = profileGradient(color);
  return (
    <LinearGradient
      colors={[from, to]}
      end={{ x: 0.85, y: 1 }}
      start={{ x: 0.15, y: 0 }}
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Text
        style={{
          color: "white",
          fontSize: Math.round(size * 0.38),
          fontWeight: "800",
        }}
      >
        {profileInitials(name)}
      </Text>
    </LinearGradient>
  );
}

function subtitle(profile: Profile) {
  const parts = [
    profile.isCurrent
      ? "On this phone"
      : profile.listens > 0
        ? `${plays.format(profile.listens)} plays`
        : "New listener",
  ];
  if (profile.scrobbling.includes("lastfm")) parts.push("Last.fm");
  if (profile.scrobbling.includes("listenbrainz")) parts.push("ListenBrainz");
  return parts.join(" · ");
}

/** Settings > Profiles: who is listening on this phone, and switching. */
export function ProfilesSettings() {
  const session = useSession();
  const queryClient = useQueryClient();
  const profiles = useQuery({
    queryKey: ["profiles", session.claims?.sub],
    queryFn: getProfiles,
  });
  const [unlocking, setUnlocking] = useState<number | null>(null);
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [pin, setPin] = useState("");
  const [color, setColor] = useState<ProfileColor>("violet");

  if (profiles.isPending) return <ActivityIndicator color="white" />;
  if (!profiles.data)
    return <Text style={styles.body}>Profiles could not be loaded.</Text>;
  const data = profiles.data;
  const list = switcherProfiles(data);

  const attempt = async (profile: Profile, value?: string) => {
    setBusy(profile.id);
    setError(null);
    const result = await session.switchProfile(
      profile.id,
      profile.unlock === "pin"
        ? { pin: value }
        : profile.unlock === "password"
          ? { password: value }
          : {},
    );
    setBusy(null);
    if (result.ok) {
      setUnlocking(null);
      setSecret("");
    } else {
      setError(result.message ?? "Could not switch profiles.");
      setSecret("");
    }
  };

  const add = async () => {
    if (!name.trim() || (pin && !validProfilePin(pin))) return;
    setBusy(-1);
    setError(null);
    try {
      await createProfile({
        name: name.trim(),
        color,
        pin: pin || undefined,
      });
      setAdding(false);
      setName("");
      setPin("");
      await queryClient.invalidateQueries({ queryKey: ["profiles"] });
    } catch (cause) {
      setError(
        (cause as { response?: { data?: { message?: string } } })?.response
          ?.data?.message ?? "Could not add the profile.",
      );
    } finally {
      setBusy(null);
    }
  };

  return (
    <View style={{ gap: 12 }}>
      <Text style={styles.heading}>Who&apos;s listening?</Text>
      <Text style={styles.body}>
        Each profile keeps its own history, favourites, Radio taste and
        scrobbling. Switching changes only this phone; the music keeps playing.
      </Text>
      {list.map((profile) => {
        const expanded = unlocking === profile.id;
        return (
          <View
            key={profile.id}
            style={[styles.card, profile.isCurrent && styles.current]}
          >
            <Pressable
              accessibilityLabel={
                profile.isCurrent
                  ? `${profile.name}, listening now`
                  : `Switch to ${profile.name}`
              }
              accessibilityRole="button"
              disabled={profile.isCurrent || busy !== null}
              onPress={() => {
                setError(null);
                if (profile.unlock === "none") void attempt(profile);
                else setUnlocking(expanded ? null : profile.id);
              }}
              style={styles.row}
            >
              <ProfileAvatar color={profile.color} name={profile.name} />
              <View style={{ flex: 1 }}>
                <Text style={styles.name}>{profile.name}</Text>
                <Text style={styles.detail}>{subtitle(profile)}</Text>
              </View>
              {busy === profile.id ? (
                <ActivityIndicator color="white" />
              ) : profile.isCurrent ? (
                <View style={styles.check}>
                  <Check color="black" size={14} strokeWidth={3} />
                </View>
              ) : profile.unlock !== "none" ? (
                <Lock color={palette.secondary} size={16} />
              ) : null}
            </Pressable>
            {expanded ? (
              <View style={{ gap: 8, marginTop: 12 }}>
                <TextInput
                  accessibilityLabel={
                    profile.unlock === "pin" ? "PIN" : "Password"
                  }
                  autoFocus
                  keyboardType={
                    profile.unlock === "pin" ? "number-pad" : "default"
                  }
                  maxLength={profile.unlock === "pin" ? 8 : 256}
                  onChangeText={setSecret}
                  onSubmitEditing={() => void attempt(profile, secret)}
                  placeholder={
                    profile.unlock === "pin" ? "PIN" : "Account password"
                  }
                  placeholderTextColor={palette.muted}
                  secureTextEntry
                  style={styles.input}
                  value={secret}
                />
                <Pressable
                  accessibilityRole="button"
                  disabled={!secret || busy !== null}
                  onPress={() => void attempt(profile, secret)}
                  style={[styles.primary, !secret && styles.disabled]}
                >
                  <Text style={styles.primaryText}>
                    Switch to {profile.name}
                  </Text>
                </Pressable>
              </View>
            ) : null}
          </View>
        );
      })}
      {error ? (
        <Text accessibilityLiveRegion="polite" style={styles.error}>
          {error}
        </Text>
      ) : null}
      {data.canCreate &&
        (adding ? (
          <View style={[styles.card, { gap: 10 }]}>
            <View style={styles.row}>
              <ProfileAvatar color={color} name={name || "?"} />
              <TextInput
                accessibilityLabel="Profile name"
                autoFocus
                maxLength={40}
                onChangeText={setName}
                placeholder="Name"
                placeholderTextColor={palette.muted}
                style={[styles.input, { flex: 1 }]}
                value={name}
              />
            </View>
            <View style={styles.colors}>
              {data.colors.map((value) => (
                <Pressable
                  accessibilityLabel={value}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: color === value }}
                  key={value}
                  onPress={() => setColor(value)}
                  style={[
                    styles.swatch,
                    color === value && styles.swatchSelected,
                  ]}
                >
                  <ProfileAvatar color={value} name={name || "?"} size={28} />
                </Pressable>
              ))}
            </View>
            <TextInput
              accessibilityLabel="PIN, optional"
              keyboardType="number-pad"
              maxLength={8}
              onChangeText={(value) => setPin(value.replace(/\D/g, ""))}
              placeholder="PIN (optional, 4 to 8 digits)"
              placeholderTextColor={palette.muted}
              secureTextEntry
              style={styles.input}
              value={pin}
            />
            <Pressable
              accessibilityRole="button"
              disabled={!name.trim() || busy !== null}
              onPress={() => void add()}
              style={[styles.primary, !name.trim() && styles.disabled]}
            >
              <Text style={styles.primaryText}>Add profile</Text>
            </Pressable>
          </View>
        ) : (
          <Pressable
            accessibilityRole="button"
            onPress={() => setAdding(true)}
            style={styles.addRow}
          >
            <View style={styles.addIcon}>
              <Plus color="white" size={18} />
            </View>
            <Text style={styles.name}>Add profile</Text>
          </Pressable>
        ))}
    </View>
  );
}

const styles = StyleSheet.create({
  heading: { color: "white", fontSize: 19, fontWeight: "800" },
  body: { color: palette.secondary, lineHeight: 21 },
  card: {
    padding: 14,
    borderRadius: 16,
    backgroundColor: palette.elevatedStrong,
    borderWidth: 1,
    borderColor: "transparent",
  },
  current: { borderColor: "rgba(217,139,95,0.45)" },
  row: { flexDirection: "row", alignItems: "center", gap: 13 },
  name: { color: "white", fontSize: 15, fontWeight: "700" },
  detail: { color: palette.secondary, fontSize: 13, marginTop: 3 },
  check: {
    width: 22,
    height: 22,
    borderRadius: 11,
    backgroundColor: palette.accent,
    alignItems: "center",
    justifyContent: "center",
  },
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
  error: { color: "#fda4af" },
  colors: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  swatch: {
    borderRadius: 18,
    padding: 2,
    borderWidth: 2,
    borderColor: "transparent",
  },
  swatchSelected: { borderColor: "white" },
  addRow: {
    minHeight: 56,
    flexDirection: "row",
    alignItems: "center",
    gap: 13,
    paddingHorizontal: 14,
  },
  addIcon: {
    width: 44,
    height: 44,
    borderRadius: 22,
    borderWidth: 1,
    borderColor: palette.borderStrong,
    borderStyle: "dashed",
    alignItems: "center",
    justifyContent: "center",
  },
});
