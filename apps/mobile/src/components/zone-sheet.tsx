import {
  anyZonePlaying,
  deviceVolumeLabel,
  estimateZonePosition,
  groupingPlan,
  orderZones,
  sleepTimerLabel,
  zoneKindLabel,
  zoneQualityLabel,
  zoneStatusLabel,
  type Zone,
} from "@parson/music-sdk";
import {
  ArrowRightLeft,
  Minus,
  Pause,
  Play,
  Plus,
  Settings2,
  SkipBack,
  SkipForward,
  Unlink,
  X,
} from "lucide-react-native";
import { useEffect, useState } from "react";
import {
  Alert,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { palette } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";
import { useZones } from "@/providers/zones-provider";

import { ZoneQueue } from "./zone-queue";
import { ZoneIconGlyph, ZoneSettingsSheet } from "./zone-settings-sheet";

function formatTime(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

function failure(error: unknown, fallback: string) {
  const data = (error as { response?: { data?: { error?: string } } })?.response
    ?.data;
  Alert.alert("Zones", data?.error?.replaceAll("_", " ") ?? fallback);
}

/** Button that opens the zone sheet; shows the zone being controlled. */
export function ZoneButton() {
  const zones = useZones();
  const [open, setOpen] = useState(false);
  const remote = zones.selectedZone;
  return (
    <>
      <Pressable
        {...immediatePressFeedback}
        accessibilityLabel={
          remote ? `Controlling ${remote.name}. Change zone` : "Choose a zone"
        }
        accessibilityRole="button"
        hitSlop={8}
        onPress={() => setOpen(true)}
        style={[styles.zoneButton, remote && styles.zoneButtonActive]}
      >
        <ZoneIconGlyph
          color={remote ? palette.success : "white"}
          zone={remote ?? { kind: "mobile" }}
        />
        {remote ? (
          <Text numberOfLines={1} style={styles.zoneButtonText}>
            {remote.name}
          </Text>
        ) : null}
      </Pressable>
      <ZoneSheet onClose={() => setOpen(false)} open={open} />
    </>
  );
}

export function ZoneSheet({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const zones = useZones();
  const insets = useSafeAreaInsets();
  const [grouping, setGrouping] = useState(false);
  const [checked, setChecked] = useState<string[]>([]);
  const [settingsFor, setSettingsFor] = useState<string | null>(null);
  const { setSheetOpen } = zones;
  useEffect(() => {
    setSheetOpen(open);
  }, [open, setSheetOpen]);
  const close = () => {
    setGrouping(false);
    setChecked([]);
    onClose();
  };
  const ordered = orderZones(zones.zones, zones.localZoneId);
  const activeId = zones.selectedZone?.id ?? zones.localZoneId;
  const active = ordered.find((zone) => zone.id === activeId) ?? null;
  const canTransfer =
    active?.source === "parson" && active.queue.song_ids.length > 0;
  const plan = groupingPlan(
    ordered.filter((zone) => checked.includes(zone.id)),
  );

  return (
    <Modal
      animationType="slide"
      onRequestClose={close}
      presentationStyle="pageSheet"
      visible={open}
    >
      <View style={[styles.sheet, { paddingBottom: insets.bottom + 12 }]}>
        <View style={styles.header}>
          <Text style={styles.title}>Zones</Text>
          <Pressable
            accessibilityLabel="Close zones"
            accessibilityRole="button"
            hitSlop={12}
            onPress={close}
          >
            <X color="white" size={24} />
          </Pressable>
        </View>
        <Text style={styles.subtitle}>
          {grouping
            ? "Pick zones to play together."
            : "Choose a zone to control, or move playback to another room."}
        </Text>
        {zones.selectedZone ? (
          <RemoteControls zone={zones.selectedZone} />
        ) : null}
        <View style={styles.toolbar}>
          <Pressable
            {...immediatePressFeedback}
            accessibilityRole="button"
            onPress={() => {
              setGrouping((value) => !value);
              setChecked(activeId ? [activeId] : []);
            }}
            style={[styles.chip, grouping && styles.chipActive]}
          >
            <Text style={[styles.chipText, grouping && styles.chipTextActive]}>
              {grouping ? "Cancel grouping" : "Group zones"}
            </Text>
          </Pressable>
          <Pressable
            {...immediatePressFeedback}
            accessibilityLabel="Pause all zones"
            accessibilityRole="button"
            accessibilityState={{ disabled: !anyZonePlaying(ordered) }}
            disabled={!anyZonePlaying(ordered)}
            onPress={() =>
              void zones
                .pauseAll()
                .catch((error) => failure(error, "Could not pause."))
            }
            style={[styles.chip, !anyZonePlaying(ordered) && styles.disabled]}
          >
            <Text style={styles.chipText}>Pause all</Text>
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={styles.list}>
          {ordered
            .filter((zone) => !grouping || zone.kind !== "linked")
            .map((zone) => {
              const isActive = zone.id === activeId;
              const isChecked = checked.includes(zone.id);
              const subtitle = zone.now_playing
                ? [zone.now_playing.title, zone.now_playing.artist]
                    .filter(Boolean)
                    .join(" · ")
                : zoneStatusLabel(zone);
              return (
                <Pressable
                  {...immediatePressFeedback}
                  accessibilityRole="button"
                  accessibilityState={{
                    selected: grouping ? isChecked : isActive,
                  }}
                  key={zone.id}
                  onPress={() => {
                    if (grouping) {
                      setChecked((current) =>
                        current.includes(zone.id)
                          ? current.filter((id) => id !== zone.id)
                          : [...current, zone.id],
                      );
                    } else {
                      zones.select(zone.id);
                    }
                  }}
                  style={[
                    styles.row,
                    (grouping ? isChecked : isActive) && styles.rowActive,
                  ]}
                >
                  <ZoneIconGlyph
                    color={
                      zone.status === "playing" ? palette.success : "white"
                    }
                    zone={zone}
                  />
                  <View style={styles.rowText}>
                    <Text numberOfLines={1} style={styles.rowTitle}>
                      {zone.name}
                      {zone.id === zones.localZoneId ? "  · This phone" : ""}
                      {zone.settings?.private ? "  · Private" : ""}
                    </Text>
                    <Text numberOfLines={1} style={styles.rowSubtitle}>
                      {subtitle}
                    </Text>
                    <Text numberOfLines={1} style={styles.rowDevice}>
                      {[
                        zone.device || zoneKindLabel(zone.kind),
                        zoneQualityLabel(zone),
                        deviceVolumeLabel(zone.volume),
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </Text>
                    {isActive && zone.output?.note ? (
                      <Text numberOfLines={3} style={styles.rowDevice}>
                        {zone.output.note}
                      </Text>
                    ) : null}
                    {zone.group ? (
                      <Text
                        numberOfLines={2}
                        style={[
                          styles.syncNote,
                          {
                            color: zone.group.synced
                              ? palette.success
                              : "#fbbf24",
                          },
                        ]}
                      >
                        {zone.group.synced
                          ? "Synced group"
                          : "Linked · not sample-synced"}
                      </Text>
                    ) : null}
                  </View>
                  {!grouping &&
                  canTransfer &&
                  active &&
                  zone.id !== active.id &&
                  zone.capabilities.transfer_target ? (
                    <Pressable
                      accessibilityLabel={`Transfer playback to ${zone.name}`}
                      accessibilityRole="button"
                      hitSlop={8}
                      onPress={() =>
                        void zones
                          .transfer(active.id, zone.id)
                          .catch((error) => failure(error, "Transfer failed."))
                      }
                      style={styles.iconButton}
                    >
                      <ArrowRightLeft color="white" size={18} />
                    </Pressable>
                  ) : null}
                  {!grouping && zone.settings ? (
                    <Pressable
                      accessibilityLabel={`${zone.name} settings`}
                      accessibilityRole="button"
                      hitSlop={8}
                      onPress={() => setSettingsFor(zone.id)}
                      style={styles.iconButton}
                    >
                      <Settings2 color="white" size={18} />
                    </Pressable>
                  ) : null}
                  {!grouping && zone.group ? (
                    <Pressable
                      accessibilityLabel={`Ungroup ${zone.name}`}
                      accessibilityRole="button"
                      hitSlop={8}
                      onPress={() =>
                        void zones
                          .ungroup(zone.group!.id)
                          .catch((error) => failure(error, "Ungroup failed."))
                      }
                      style={styles.iconButton}
                    >
                      <Unlink color="white" size={18} />
                    </Pressable>
                  ) : null}
                </Pressable>
              );
            })}
        </ScrollView>
        {grouping ? (
          <View style={styles.groupFooter}>
            <Text
              style={[
                styles.planText,
                {
                  color: plan.allowed
                    ? plan.synced
                      ? palette.success
                      : "#fbbf24"
                    : palette.secondary,
                },
              ]}
            >
              {plan.reason}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityState={{ disabled: !plan.allowed }}
              disabled={!plan.allowed}
              onPress={() =>
                void zones
                  .group(checked)
                  .then(() => setGrouping(false))
                  .catch((error) => failure(error, "Grouping failed."))
              }
              style={[styles.primary, !plan.allowed && styles.disabled]}
            >
              <Text style={styles.primaryText}>Save group</Text>
            </Pressable>
          </View>
        ) : null}
      </View>
      <ZoneSettingsSheet
        onClose={() => setSettingsFor(null)}
        zone={zones.zones.find((zone) => zone.id === settingsFor) ?? null}
      />
    </Modal>
  );
}

function RemoteControls({ zone }: { zone: Zone }) {
  const zones = useZones();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (zone.status !== "playing") return;
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [zone.status]);
  const send = (command: Parameters<typeof zones.send>[1]) =>
    void zones
      .send(zone.id, command)
      .catch((error) => failure(error, "The zone did not respond."));
  const position = estimateZonePosition(zone, zones.fetchedAt, now) / 1000;
  const duration =
    (zone.duration_ms || zone.now_playing?.duration_ms || 0) / 1000;
  return (
    <View style={styles.remote}>
      <Text numberOfLines={1} style={styles.remoteTitle}>
        {zone.now_playing?.title ?? `Nothing playing in ${zone.name}`}
      </Text>
      <Text numberOfLines={1} style={styles.rowSubtitle}>
        {zone.now_playing?.artist || zoneStatusLabel(zone)}
        {duration > 0
          ? `  ·  ${formatTime(position)} / ${formatTime(duration)}`
          : ""}
      </Text>
      {zone.sleep_timer ? (
        <Text
          numberOfLines={1}
          style={[styles.syncNote, { color: palette.secondary }]}
        >
          {sleepTimerLabel(zone.sleep_timer, now)}
        </Text>
      ) : null}
      <View style={styles.transport}>
        <Pressable
          accessibilityLabel="Previous"
          accessibilityRole="button"
          hitSlop={10}
          onPress={() => send({ command: "previous" })}
        >
          <SkipBack color="white" fill="white" size={26} />
        </Pressable>
        <Pressable
          accessibilityLabel={zone.status === "playing" ? "Pause" : "Play"}
          accessibilityRole="button"
          onPress={() =>
            send({ command: zone.status === "playing" ? "pause" : "play" })
          }
          style={styles.playButton}
        >
          {zone.status === "playing" ? (
            <Pause color="black" fill="black" size={24} />
          ) : (
            <Play color="black" fill="black" size={24} />
          )}
        </Pressable>
        <Pressable
          accessibilityLabel="Next"
          accessibilityRole="button"
          hitSlop={10}
          onPress={() => send({ command: "next" })}
        >
          <SkipForward color="white" fill="white" size={26} />
        </Pressable>
      </View>
      {zone.volume && zone.capabilities.volume ? (
        <View style={styles.volume}>
          <Pressable
            accessibilityLabel="Volume down"
            accessibilityRole="button"
            hitSlop={8}
            onPress={() => send({ command: "adjust_volume", delta: -4 })}
            style={styles.iconButton}
          >
            <Minus color="white" size={18} />
          </Pressable>
          <Text style={styles.volumeText}>
            {zone.volume.muted
              ? "Muted"
              : `Volume ${zone.volume.level}${
                  zone.volume.comfort != null
                    ? ` · comfort ${zone.volume.comfort}`
                    : ""
                }${zone.volume.max < 100 ? ` · max ${zone.volume.max}` : ""}`}
          </Text>
          <Pressable
            accessibilityLabel="Volume up"
            accessibilityRole="button"
            hitSlop={8}
            onPress={() => send({ command: "adjust_volume", delta: 4 })}
            style={styles.iconButton}
          >
            <Plus color="white" size={18} />
          </Pressable>
        </View>
      ) : null}
      {zone.capabilities.queue && zone.queue.song_ids.length > 0 ? (
        <ZoneQueue
          send={(command) =>
            zones.send(zone.id, command).catch((error) => {
              failure(error, "The zone did not change its queue.");
              throw error;
            })
          }
          zone={zone}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  sheet: {
    backgroundColor: palette.background,
    flex: 1,
    paddingHorizontal: 20,
    paddingTop: 20,
  },
  header: {
    alignItems: "center",
    flexDirection: "row",
    justifyContent: "space-between",
  },
  title: { color: "white", fontSize: 24, fontWeight: "700" },
  subtitle: { color: palette.secondary, fontSize: 13, marginTop: 6 },
  toolbar: { flexDirection: "row", gap: 8, marginTop: 14 },
  chip: {
    borderColor: palette.borderStrong,
    borderRadius: 999,
    borderWidth: 1,
    paddingHorizontal: 14,
    paddingVertical: 7,
  },
  chipActive: { backgroundColor: "white", borderColor: "white" },
  chipText: { color: "white", fontSize: 13, fontWeight: "600" },
  chipTextActive: { color: "black" },
  list: { gap: 8, paddingVertical: 14 },
  row: {
    alignItems: "center",
    borderColor: palette.border,
    borderRadius: 14,
    borderWidth: 1,
    flexDirection: "row",
    gap: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  rowActive: {
    backgroundColor: palette.elevatedStrong,
    borderColor: palette.borderStrong,
  },
  rowText: { flex: 1, minWidth: 0 },
  rowTitle: { color: "white", fontSize: 15, fontWeight: "600" },
  rowSubtitle: { color: palette.secondary, fontSize: 12, marginTop: 2 },
  rowDevice: {
    color: palette.muted,
    fontSize: 11,
    marginTop: 2,
  },
  syncNote: { fontSize: 11, marginTop: 3 },
  iconButton: {
    alignItems: "center",
    borderColor: palette.border,
    borderRadius: 999,
    borderWidth: 1,
    height: 34,
    justifyContent: "center",
    width: 34,
  },
  remote: {
    backgroundColor: palette.elevated,
    borderColor: palette.border,
    borderRadius: 16,
    borderWidth: 1,
    marginTop: 14,
    padding: 16,
  },
  remoteTitle: { color: "white", fontSize: 16, fontWeight: "700" },
  transport: {
    alignItems: "center",
    flexDirection: "row",
    gap: 36,
    justifyContent: "center",
    marginTop: 14,
  },
  playButton: {
    alignItems: "center",
    backgroundColor: "white",
    borderRadius: 999,
    height: 52,
    justifyContent: "center",
    width: 52,
  },
  volume: {
    alignItems: "center",
    flexDirection: "row",
    gap: 16,
    justifyContent: "center",
    marginTop: 14,
  },
  volumeText: {
    color: "white",
    fontSize: 13,
    minWidth: 90,
    textAlign: "center",
  },
  groupFooter: {
    borderColor: palette.border,
    borderTopWidth: 1,
    gap: 10,
    paddingTop: 12,
  },
  planText: { fontSize: 13, lineHeight: 18 },
  primary: {
    alignItems: "center",
    backgroundColor: "white",
    borderRadius: 999,
    paddingVertical: 12,
  },
  primaryText: { color: "black", fontSize: 15, fontWeight: "700" },
  disabled: { opacity: 0.4 },
  zoneButton: {
    alignItems: "center",
    borderRadius: 999,
    flexDirection: "row",
    gap: 6,
    height: 32,
    justifyContent: "center",
    maxWidth: 150,
    minWidth: 32,
    paddingHorizontal: 6,
  },
  zoneButtonActive: { backgroundColor: "rgba(34,197,94,0.15)" },
  zoneButtonText: { color: palette.success, fontSize: 12, fontWeight: "600" },
});
