import {
  SIGNAL_CLIPPING_COLOR,
  buildSignalPath,
  defaultDspSettings,
  getNativeOutputSession,
  getNativeOutputs,
  normalizeDspSettings,
  playOnNativeOutput,
  sendNativeOutputCommand,
  signalLightColor,
  signalLightLabel,
  signalQualityColors,
  updateNativeOutputDsp,
  type DspSettings,
  type NativeOutputCommand,
  type NativeOutputs,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { palette } from "@/constants/colors";
import { useState } from "react";
import {
  serverDspStatusLine,
  serverDspSummary,
  serverOutputSummary,
  serverSessionLine,
  unsupportedRates,
} from "@/lib/server-outputs";
import { DspControls, QualityGlyph } from "./signal-path";

const sessionKey = ["native-output-session"] as const;

/**
 * The server's own devices, for bit-perfect playback. A phone can only
 * play through its system mixer, so this hands the current track to the
 * computer running Parson, which opens its DAC exclusively; the phone
 * then acts as the remote.
 */
export function ServerOutputs({
  songId,
  positionSeconds,
  onHandOff,
}: {
  songId: string | null;
  positionSeconds: number;
  /** Pauses local playback once the server has taken over. */
  onHandOff: () => void;
}) {
  const queryClient = useQueryClient();
  const outputs = useQuery({
    queryKey: ["native-outputs"],
    queryFn: getNativeOutputs,
    staleTime: 30_000,
    retry: 1,
  });
  const session = useQuery({
    queryKey: sessionKey,
    queryFn: getNativeOutputSession,
    refetchInterval: 1_000,
    retry: false,
  });
  const play = useMutation({
    mutationFn: (deviceId: string) =>
      playOnNativeOutput({
        device_id: deviceId,
        song_id: songId ?? "",
        position_ms: positionSeconds * 1000,
      }),
    onSuccess: (started) => {
      queryClient.setQueryData(sessionKey, started);
      onHandOff();
    },
  });
  const command = useMutation({
    mutationFn: (name: NativeOutputCommand) => sendNativeOutputCommand(name),
    onSuccess: (next) => queryClient.setQueryData(sessionKey, next),
  });
  // Edits show at once; the server saves them and restarts the track.
  const [draft, setDraft] = useState<{
    id: string;
    settings: DspSettings;
  } | null>(null);
  const saveDsp = useMutation({
    mutationFn: (input: { id: string; settings: DspSettings }) =>
      updateNativeOutputDsp(input.id, input.settings),
    onSuccess: (saved) => {
      queryClient.setQueryData<NativeOutputs>(["native-outputs"], (old) =>
        old
          ? {
              ...old,
              devices: old.devices.map((item) =>
                item.id === saved.id ? saved : item,
              ),
            }
          : old,
      );
      void queryClient.invalidateQueries({ queryKey: sessionKey });
      // Drop the draft once the server holds it (a newer edit stays).
      setDraft((pending) =>
        pending?.id === saved.id &&
        JSON.stringify(normalizeDspSettings(pending.settings)) ===
          JSON.stringify(normalizeDspSettings(saved.dsp))
          ? null
          : pending,
      );
    },
  });
  const devices = outputs.data?.devices ?? [];
  if (!outputs.data?.supported || !devices.length) return null;
  const current = session.data ?? null;
  const focused =
    devices.find((device) => device.id === current?.device_id) ?? devices[0]!;
  const focusedDsp =
    draft?.id === focused.id
      ? draft.settings
      : focused.dsp
        ? normalizeDspSettings(focused.dsp)
        : defaultDspSettings();
  const status = current?.signal_path.processing?.status ?? null;
  const statusLine = serverDspStatusLine(status);
  const path = current
    ? buildSignalPath(current.signal_path, {
        platform: "web",
        engine: "native",
        playbackRate: 1,
        preservePitch: true,
        reverbWet: 0,
        output: { kind: "native", name: current.device_name },
      })
    : null;

  return (
    <View style={styles.section}>
      <Text style={styles.title}>Server outputs</Text>
      <Text style={styles.note}>
        Play this track on a device attached to the computer running Parson,
        opened in exclusive mode for a bit-perfect path.
      </Text>
      {current && path ? (
        <View style={styles.session}>
          <View style={styles.row}>
            <QualityGlyph
              clipping={path.clipping}
              quality={path.quality}
              size={12}
            />
            <Text style={styles.sessionName}>{current.device_name}</Text>
            <Text
              style={[styles.sessionQuality, { color: signalLightColor(path) }]}
            >
              {signalLightLabel(path)}
              {path.bitPerfect
                ? " · bit-perfect"
                : path.exactAfterProcessing
                  ? " · bit-exact after DSP"
                  : ""}
            </Text>
          </View>
          <Text style={styles.detail}>{serverSessionLine(current)}</Text>
          {statusLine ? (
            <Text
              style={[
                styles.detail,
                status?.clipping && { color: SIGNAL_CLIPPING_COLOR },
              ]}
            >
              {statusLine}
            </Text>
          ) : null}
          <View style={styles.row}>
            {current.state === "paused" ? (
              <SmallButton
                label="Resume"
                onPress={() => command.mutate("resume")}
              />
            ) : (
              <SmallButton
                label="Pause"
                onPress={() => command.mutate("pause")}
              />
            )}
            <SmallButton label="Stop" onPress={() => command.mutate("stop")} />
          </View>
        </View>
      ) : null}
      {devices.map((device) => (
        <View key={device.id} style={styles.device}>
          <View style={styles.deviceText}>
            <Text numberOfLines={1} style={styles.deviceName}>
              {device.name}
            </Text>
            <Text numberOfLines={1} style={styles.detail}>
              {device.mode_label} · {serverOutputSummary(device)}
            </Text>
            {unsupportedRates(device) ? (
              <Text numberOfLines={1} style={styles.unsupported}>
                Converted: {unsupportedRates(device)}
              </Text>
            ) : null}
            {serverDspSummary(device) ? (
              <Text
                numberOfLines={1}
                style={[styles.detail, { color: signalQualityColors.enhanced }]}
              >
                Server DSP: {serverDspSummary(device)}
              </Text>
            ) : null}
          </View>
          <SmallButton
            disabled={!songId || !device.formats || play.isPending}
            label="Play here"
            onPress={() => play.mutate(device.id)}
          />
        </View>
      ))}
      <DspControls
        crossfeed={focused.dsp_support ? focused.dsp_support.crossfeed : null}
        onChange={(settings) => {
          setDraft({ id: focused.id, settings });
          saveDsp.mutate({ id: focused.id, settings });
        }}
        settings={focusedDsp}
        title={`DSP on ${focused.name} (server, 64-bit)`}
        unavailable={
          !focused.dsp_support
            ? "This server does not run a DSP engine for its outputs."
            : focused.dsp_support.available
              ? null
              : focused.dsp_support.reason
        }
      />
      {saveDsp.error ? (
        <Text style={styles.unsupported}>
          {saveDsp.error instanceof Error
            ? saveDsp.error.message
            : "The server did not save the DSP settings."}
        </Text>
      ) : null}
      {play.error ? (
        <Text style={styles.unsupported}>
          {play.error instanceof Error
            ? play.error.message
            : "The server could not play this track."}
        </Text>
      ) : null}
    </View>
  );
}

function SmallButton({
  disabled,
  label,
  onPress,
}: {
  disabled?: boolean;
  label: string;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityLabel={label}
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={[styles.button, disabled && styles.buttonDisabled]}
    >
      <Text style={styles.buttonText}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  section: {
    marginTop: 14,
    paddingTop: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: palette.borderStrong,
    gap: 8,
  },
  title: { color: "white", fontSize: 15, fontWeight: "700" },
  note: { color: palette.secondary, fontSize: 12, lineHeight: 17 },
  session: {
    gap: 6,
    padding: 10,
    borderRadius: 12,
    backgroundColor: "rgba(178,107,255,0.08)",
  },
  row: { flexDirection: "row", alignItems: "center", gap: 8 },
  sessionName: { flex: 1, color: "white", fontSize: 13, fontWeight: "600" },
  sessionQuality: { fontSize: 12, fontWeight: "700" },
  device: { flexDirection: "row", alignItems: "center", gap: 10 },
  deviceText: { flex: 1, minWidth: 0 },
  deviceName: { color: "white", fontSize: 13, fontWeight: "600" },
  detail: { color: palette.secondary, fontSize: 12 },
  unsupported: { color: "#f87171", fontSize: 11 },
  button: {
    minHeight: 32,
    justifyContent: "center",
    paddingHorizontal: 12,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.borderStrong,
  },
  buttonDisabled: { opacity: 0.4 },
  buttonText: { color: "white", fontSize: 12, fontWeight: "600" },
});
