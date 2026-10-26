import {
  SIGNAL_CLIPPING_COLOR,
  DSD_STRATEGIES,
  getConvolutionFilters,
  normalizeDspSettings,
  signalLightColor,
  signalLightLabel,
  signalQualityColors,
  zoneSignalPath,
  type DsdStrategy,
  type DspSettings,
  type ZoneCommand,
  type ZoneSettingsPatch,
} from "@parson/music-sdk";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { palette } from "@/constants/colors";
import { useState } from "react";
import {
  networkPlayerLine,
  networkPlayerNote,
  serverDriverLine,
  serverDsdLine,
  serverDspStatusLine,
  serverDspSummary,
  serverHardwareLine,
  serverOutputs,
  serverOutputSummary,
  serverSessionLine,
  serverTransportLine,
  unsupportedRates,
} from "@/lib/server-outputs";
import { useZones } from "@/providers/zones-provider";
import { DspControls, QualityGlyph } from "./signal-path";

function errorMessage(error: unknown, fallback: string) {
  const data = (error as { response?: { data?: { error?: unknown } } })
    ?.response?.data;
  if (typeof data?.error === "string") return data.error.replaceAll("_", " ");
  return error instanceof Error && error.message ? error.message : fallback;
}

/**
 * The zones the server plays itself: devices on the computer running
 * Parson (opened exclusively for a bit-perfect path), Parson Bridges'
 * devices and network players (UPnP, OpenHome, AirPlay) it streams to
 * after its own DSP. A phone can only play through its system mixer, so
 * "Play here" moves the queue to that zone and the phone becomes the
 * remote. Each zone keeps its own DSP chain and DSD strategy.
 */
export function ServerOutputs({
  onHandOff,
}: {
  /** Pauses local playback once the zone has taken over. */
  onHandOff: () => void;
}) {
  const zones = useZones();
  const filters = useQuery({
    queryKey: ["convolution-filters"],
    queryFn: getConvolutionFilters,
    staleTime: 60_000,
    retry: 1,
  });
  const play = useMutation({
    mutationFn: async (zoneId: string) => {
      if (!zones.localZoneId)
        throw new Error("This phone has not joined the zone list yet.");
      await zones.transfer(zones.localZoneId, zoneId);
    },
    onSuccess: onHandOff,
  });
  const command = useMutation({
    mutationFn: (input: { zoneId: string; command: ZoneCommand }) =>
      zones.send(input.zoneId, input.command),
  });
  // Edits show at once; the server saves them and replans the stream.
  const [draft, setDraft] = useState<{
    id: string;
    settings: DspSettings;
  } | null>(null);
  const save = useMutation({
    mutationFn: (input: { id: string; patch: ZoneSettingsPatch }) =>
      zones.updateSettings(input.id, input.patch),
    onSettled: () => setDraft(null),
  });
  const outputs = serverOutputs(zones.zones);
  if (!outputs.length) return null;
  const selected =
    outputs.find((output) => output.zone.id === zones.selectedZone?.id) ?? null;
  const focused = selected ?? outputs[0]!;
  const settings = focused.zone.settings;
  const focusedDsp =
    draft?.id === focused.zone.id
      ? draft.settings
      : normalizeDspSettings(settings?.dsp);
  const path = selected ? zoneSignalPath(selected.zone) : null;
  const status = path?.processing?.status ?? null;
  const statusLine = serverDspStatusLine(status);
  const support = focused.endpoint.dsp_support;

  return (
    <View style={styles.section}>
      <Text style={styles.title}>Server outputs</Text>
      <Text style={styles.note}>
        Play on a device attached to the computer running Parson, opened in
        exclusive mode for a bit-perfect path, or on a network player (UPnP,
        OpenHome, AirPlay) the server streams to after its own DSP. The queue
        moves there and this phone becomes the remote.
      </Text>
      {selected && path ? (
        <View style={styles.session}>
          <View style={styles.row}>
            <QualityGlyph
              clipping={path.clipping}
              quality={path.quality}
              size={12}
            />
            <Text style={styles.sessionName}>{selected.zone.name}</Text>
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
          <Text style={styles.detail}>{serverSessionLine(selected.zone)}</Text>
          {serverTransportLine(path) ? (
            <Text style={styles.detail}>{serverTransportLine(path)}</Text>
          ) : null}
          {serverDriverLine(path) ? (
            <Text
              style={[
                styles.detail,
                {
                  color: path.hardwareVerified
                    ? path.quality === "lossless"
                      ? signalQualityColors.lossless
                      : palette.secondary
                    : signalQualityColors.high_quality,
                },
              ]}
            >
              {serverDriverLine(path)}
            </Text>
          ) : null}
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
            {selected.zone.status === "paused" ? (
              <SmallButton
                label="Resume"
                onPress={() =>
                  command.mutate({
                    zoneId: selected.zone.id,
                    command: { command: "play" },
                  })
                }
              />
            ) : (
              <SmallButton
                label="Pause"
                onPress={() =>
                  command.mutate({
                    zoneId: selected.zone.id,
                    command: { command: "pause" },
                  })
                }
              />
            )}
            <SmallButton
              label="Stop"
              onPress={() =>
                command.mutate({
                  zoneId: selected.zone.id,
                  command: { command: "stop" },
                })
              }
            />
          </View>
        </View>
      ) : null}
      {outputs.map(({ zone, endpoint }) => (
        <View key={zone.id} style={styles.device}>
          <View style={styles.deviceText}>
            <Text numberOfLines={1} style={styles.deviceName}>
              {zone.name}
            </Text>
            {networkPlayerLine(endpoint) ? (
              <Text numberOfLines={1} style={styles.network}>
                {networkPlayerLine(endpoint)}
              </Text>
            ) : null}
            <Text numberOfLines={1} style={styles.detail}>
              {[endpoint.mode_label, serverOutputSummary(endpoint)]
                .filter(Boolean)
                .join(" · ")}
            </Text>
            {networkPlayerNote(endpoint) ? (
              <Text numberOfLines={2} style={styles.tested}>
                {networkPlayerNote(endpoint)}
              </Text>
            ) : null}
            {unsupportedRates(endpoint) ? (
              <Text numberOfLines={1} style={styles.unsupported}>
                Converted: {unsupportedRates(endpoint)}
              </Text>
            ) : null}
            {serverHardwareLine(endpoint) ? (
              <Text numberOfLines={1} style={styles.detail}>
                {serverHardwareLine(endpoint)}
              </Text>
            ) : null}
            {zone.settings?.supports.dsd &&
            serverDsdLine(endpoint, zone.settings.dsd_strategy) ? (
              <Text numberOfLines={1} style={styles.detail}>
                {serverDsdLine(endpoint, zone.settings.dsd_strategy)}
              </Text>
            ) : null}
            {serverDspSummary(zone.settings?.dsp) ? (
              <Text
                numberOfLines={1}
                style={[styles.detail, { color: signalQualityColors.enhanced }]}
              >
                Server DSP: {serverDspSummary(zone.settings?.dsp)}
              </Text>
            ) : null}
          </View>
          <SmallButton
            disabled={
              zone.id === selected?.zone.id ||
              !endpoint.formats ||
              play.isPending
            }
            label={zone.id === selected?.zone.id ? "Playing here" : "Play here"}
            onPress={() => play.mutate(zone.id)}
          />
        </View>
      ))}
      {settings?.supports.dsd && focused.endpoint.formats?.dsd_rates ? (
        <View style={styles.strategies}>
          <Text style={styles.detail}>DSD playback on {focused.zone.name}</Text>
          <View style={styles.chips}>
            {DSD_STRATEGIES.map((strategy) => {
              const chosen =
                (settings.dsd_strategy ?? "auto") === strategy.value;
              return (
                <Pressable
                  accessibilityLabel={`DSD playback strategy: ${strategy.label}`}
                  accessibilityRole="button"
                  accessibilityState={{ selected: chosen }}
                  disabled={save.isPending}
                  key={strategy.value}
                  onPress={() =>
                    save.mutate({
                      id: focused.zone.id,
                      patch: { dsd_strategy: strategy.value as DsdStrategy },
                    })
                  }
                  style={[styles.button, chosen && styles.chosen]}
                >
                  <Text style={styles.buttonText}>{strategy.label}</Text>
                </Pressable>
              );
            })}
          </View>
        </View>
      ) : null}
      {settings?.supports.dsp ? (
        <DspControls
          crossfeed={support ? support.crossfeed : null}
          filters={
            filters.data
              ? filters.data.map((filter) => ({
                  id: filter.id,
                  name: filter.name,
                  peakGainDb: filter.summary.peak_gain_db,
                }))
              : null
          }
          onChange={(next) => {
            setDraft({ id: focused.zone.id, settings: next });
            save.mutate({ id: focused.zone.id, patch: { dsp: next } });
          }}
          settings={focusedDsp}
          title={`DSP on ${focused.zone.name} (server, 64-bit)`}
          unavailable={support && !support.available ? support.reason : null}
        />
      ) : null}
      {save.error ? (
        <Text style={styles.unsupported}>
          {errorMessage(save.error, "The server did not save the setting.")}
        </Text>
      ) : null}
      {play.error ? (
        <Text style={styles.unsupported}>
          {errorMessage(play.error, "The zone could not take over.")}
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
  network: { color: "#c4b5fd", fontSize: 12 },
  tested: { color: "rgba(252,211,77,0.85)", fontSize: 11, lineHeight: 15 },
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
  chosen: {
    borderColor: signalQualityColors.lossless,
    backgroundColor: "rgba(178,107,255,0.16)",
  },
  strategies: { gap: 6 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  buttonText: { color: "white", fontSize: 12, fontWeight: "600" },
});
