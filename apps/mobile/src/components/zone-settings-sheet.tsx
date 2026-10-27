import {
  CROSSFADE_CHOICES,
  DSD_RATES,
  DSD_STRATEGIES,
  LEVELING_ADJUSTMENT_RANGE,
  LEVELING_TARGET_RANGE,
  SAMPLE_RATE_CHOICES,
  ZONE_ICONS,
  deviceVolumeLabel,
  dsdRateLabel,
  formatSampleRate,
  levelingModeLabel,
  streamQuality,
  streamQualityText,
  volumeModeLabel,
  zoneIconName,
  zoneSettingsDraft,
  zoneSettingsPatch,
  zoneSettingsProblem,
  type Zone,
  type ZoneIconName,
  type ZoneLevelingMode,
  type DsdStrategy,
  type ZoneSettingsDraft,
  type ZoneStreamInfo,
  type ZoneStreamQuality,
  type ZoneVolumeControl,
  type ZoneVolumeMode,
} from "@parson/music-sdk";
import {
  AudioWaveform,
  Bed,
  Briefcase,
  Car,
  CookingPot,
  Disc3,
  Headphones,
  Laptop,
  Lock,
  Minus,
  Monitor,
  Music,
  Plus,
  Radio,
  Smartphone,
  Sofa,
  Speaker,
  Tablet,
  Trees,
  Tv,
  X,
} from "lucide-react-native";
import { useMemo, useState, type ReactNode } from "react";
import {
  Alert,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { palette } from "@/constants/colors";
import { DspControls } from "./signal-path";
import { signedLabel, steppedValue } from "@/lib/zones";
import { useZones } from "@/providers/zones-provider";

const ICONS: Record<ZoneIconName, typeof Speaker> = {
  speaker: Speaker,
  headphones: Headphones,
  monitor: Monitor,
  laptop: Laptop,
  phone: Smartphone,
  tablet: Tablet,
  tv: Tv,
  radio: Radio,
  turntable: Disc3,
  amplifier: AudioWaveform,
  car: Car,
  bed: Bed,
  sofa: Sofa,
  kitchen: CookingPot,
  office: Briefcase,
  garden: Trees,
  music: Music,
};

const LEVELING_MODES: ZoneLevelingMode[] = ["off", "auto", "track", "album"];

/** A zone's icon (custom or its kind's). */
export function ZoneIconGlyph({
  zone,
  color,
  size = 20,
}: {
  zone: Pick<Zone, "icon" | "kind">;
  color: string;
  size?: number;
}) {
  const Icon = ICONS[zoneIconName(zone)] ?? Music;
  return <Icon color={color} size={size} />;
}

function failure(error: unknown) {
  const code = (error as { response?: { data?: { error?: string } } })?.response
    ?.data?.error;
  Alert.alert(
    "Zone settings",
    code === "set_private_on_the_device_itself"
      ? "A zone can only be made private from the device it runs on."
      : (code?.replaceAll("_", " ") ?? "Could not save the zone settings."),
  );
}

/** Roon-style Zone settings for any zone this phone can see. */
export function ZoneSettingsSheet({
  zone,
  onClose,
}: {
  zone: Zone | null;
  onClose: () => void;
}) {
  return (
    <Modal
      animationType="slide"
      onRequestClose={onClose}
      presentationStyle="pageSheet"
      visible={Boolean(zone)}
    >
      {zone ? (
        <SettingsForm key={zone.id} onClose={onClose} zone={zone} />
      ) : null}
    </Modal>
  );
}

function SettingsForm({ zone, onClose }: { zone: Zone; onClose: () => void }) {
  const zones = useZones();
  const insets = useSafeAreaInsets();
  const initial = useMemo(() => zoneSettingsDraft(zone), [zone]);
  const [draft, setDraft] = useState<ZoneSettingsDraft>(initial);
  const [saving, setSaving] = useState(false);
  const update = (change: Partial<ZoneSettingsDraft>) =>
    setDraft((current) => ({ ...current, ...change }));
  const supports = zone.settings?.supports;
  const patch = zoneSettingsPatch(initial, draft);
  const changed = Object.keys(patch).length > 0;
  const problem = zoneSettingsProblem(draft);
  const isThisPhone = zone.id === zones.localZoneId;
  const dspSupport =
    zone.endpoints?.find((endpoint) => endpoint.dsp_support)?.dsp_support ??
    null;
  const volumeModes = supports?.volume_modes ?? [];
  const stream = zone.output?.stream;
  const defaultName = zone.settings?.default_name ?? zone.name;
  const min = draft.safety_min_volume ?? 0;
  const max = draft.safety_max_volume ?? 100;
  const clampComfort = (low: number, high: number) =>
    draft.comfort_volume === null
      ? null
      : Math.min(Math.max(draft.comfort_volume, low), high);

  const save = () => {
    if (!changed || problem) return;
    setSaving(true);
    void zones
      .updateSettings(zone.id, patch)
      .then(onClose)
      .catch(failure)
      .finally(() => setSaving(false));
  };

  return (
    <View style={[styles.sheet, { paddingBottom: insets.bottom + 12 }]}>
      <View style={styles.header}>
        <View style={styles.headerText}>
          <Text style={styles.title}>Zone settings</Text>
          <Text numberOfLines={1} style={styles.subtitle}>
            {zone.device}
            {isThisPhone ? " · This phone" : ""}
          </Text>
        </View>
        <Pressable
          accessibilityLabel="Close zone settings"
          accessibilityRole="button"
          hitSlop={12}
          onPress={onClose}
        >
          <X color="white" size={24} />
        </Pressable>
      </View>
      <ScrollView contentContainerStyle={styles.content}>
        <Section title="Zone">
          <Text style={styles.label}>Zone name</Text>
          <TextInput
            accessibilityLabel="Zone name"
            maxLength={80}
            onChangeText={(name) => update({ name })}
            placeholder={defaultName}
            placeholderTextColor={palette.muted}
            style={styles.input}
            value={draft.name}
          />
          <View style={styles.inline}>
            <Text style={styles.note}>Device name: {defaultName}</Text>
            {draft.name.trim() !== defaultName ? (
              <Pressable
                accessibilityRole="button"
                onPress={() => update({ name: defaultName })}
              >
                <Text style={styles.link}>Use device name</Text>
              </Pressable>
            ) : null}
          </View>
          <Text style={[styles.label, styles.spaced]}>Zone icon</Text>
          <View style={styles.icons}>
            {([null, ...ZONE_ICONS] as (ZoneIconName | null)[]).map((icon) => {
              const selected = draft.icon === icon;
              return (
                <Pressable
                  accessibilityLabel={icon ? `Icon ${icon}` : "Device icon"}
                  accessibilityRole="button"
                  accessibilityState={{ selected }}
                  key={icon ?? "device"}
                  onPress={() => update({ icon })}
                  style={[styles.iconChoice, selected && styles.choiceActive]}
                >
                  <ZoneIconGlyph
                    color={selected ? "black" : "white"}
                    size={18}
                    zone={{ kind: zone.kind, icon: icon ?? undefined }}
                  />
                </Pressable>
              );
            })}
          </View>
        </Section>

        <Section title="Zone options">
          <Text style={styles.label}>Crossfade time</Text>
          {supports?.crossfade ? (
            <Chips
              onSelect={(value) => update({ crossfade_seconds: Number(value) })}
              options={CROSSFADE_CHOICES.map((seconds) => ({
                value: String(seconds),
                label: seconds ? `${seconds} s` : "Off",
              }))}
              selected={String(draft.crossfade_seconds)}
            />
          ) : (
            <Text style={styles.note}>Not available on this zone.</Text>
          )}
          <Text style={[styles.label, styles.spaced]}>Volume leveling</Text>
          {supports?.leveling ? (
            <>
              <Chips
                onSelect={(value) =>
                  update({ leveling: value as ZoneLevelingMode })
                }
                options={LEVELING_MODES.map((mode) => ({
                  value: mode,
                  label: levelingModeLabel(mode),
                }))}
                selected={draft.leveling}
              />
              {draft.leveling !== "off" ? (
                <>
                  <Stepper
                    label="Target"
                    onChange={(leveling_target_lufs) =>
                      update({ leveling_target_lufs })
                    }
                    range={LEVELING_TARGET_RANGE}
                    text={`${draft.leveling_target_lufs} LUFS`}
                    value={draft.leveling_target_lufs}
                  />
                  <Stepper
                    label="Adjustment"
                    onChange={(leveling_adjustment_db) =>
                      update({ leveling_adjustment_db })
                    }
                    range={LEVELING_ADJUSTMENT_RANGE}
                    text={signedLabel(draft.leveling_adjustment_db, "dB")}
                    value={draft.leveling_adjustment_db}
                  />
                </>
              ) : null}
            </>
          ) : (
            <Text style={styles.note}>Not available on this zone.</Text>
          )}
        </Section>

        {supports?.volume_limits !== false ? (
          <Section title="Volume limits">
            <View style={styles.switchRow}>
              <View style={styles.switchText}>
                <Text style={styles.rowTitle}>Comfort limit</Text>
                <Text style={styles.note}>
                  Sliders stop here; + can still go past it.
                </Text>
              </View>
              <Switch
                accessibilityLabel="Comfort limit"
                onValueChange={(on) =>
                  update({
                    comfort_volume: on
                      ? Math.min(Math.max(50, min), max)
                      : null,
                  })
                }
                value={draft.comfort_volume !== null}
              />
            </View>
            {draft.comfort_volume !== null ? (
              <Stepper
                label="Comfort"
                onChange={(comfort_volume) => update({ comfort_volume })}
                range={{ min, max }}
                step={5}
                text={String(draft.comfort_volume)}
                value={draft.comfort_volume}
              />
            ) : null}
            <Text style={[styles.rowTitle, styles.spaced]}>Safety limits</Text>
            <Text style={styles.note}>
              Hard limits: nothing sets the volume outside this range.
            </Text>
            <Stepper
              label="Minimum"
              onChange={(low) => {
                const high = Math.max(max, low + 1);
                update({
                  safety_min_volume: low > 0 ? low : null,
                  safety_max_volume: high < 100 ? high : null,
                  comfort_volume: clampComfort(low, high),
                });
              }}
              range={{ min: 0, max: 99 }}
              step={5}
              text={String(min)}
              value={min}
            />
            <Stepper
              label="Maximum"
              onChange={(high) => {
                const low = Math.min(min, high - 1);
                update({
                  safety_min_volume: low > 0 ? low : null,
                  safety_max_volume: high < 100 ? high : null,
                  comfort_volume: clampComfort(low, high),
                });
              }}
              range={{ min: 1, max: 100 }}
              step={5}
              text={String(max)}
              value={max}
            />
            <View style={styles.inline}>
              <Text style={styles.rowTitle}>Device range</Text>
              <Text style={styles.note}>
                {deviceVolumeLabel(zone.volume) ??
                  `${zone.volume?.device_min ?? 0}–${zone.volume?.device_max ?? 100} · reported by the device`}
              </Text>
            </View>
          </Section>
        ) : null}

        {supports?.volume_control ? (
          <Section title="Home Assistant">
            <Text style={styles.label}>Volume handled by</Text>
            <Chips
              onSelect={(value) =>
                update({ volume_control: value as ZoneVolumeControl })
              }
              options={[
                { value: "parson", label: "Mujin Music" },
                { value: "home_assistant", label: "Home Assistant" },
              ]}
              selected={draft.volume_control}
            />
            <Text style={styles.note}>
              {draft.volume_control === "home_assistant"
                ? "Volume changes go to Home Assistant as events for an automation to carry out, such as an amplifier on an IR blaster. Mujin Music leaves the audio at full level."
                : "Mujin Music sets the volume itself."}
            </Text>
          </Section>
        ) : null}

        {supports?.dsp ? (
          <Section title="DSP">
            <DspControls
              crossfeed={dspSupport ? dspSupport.crossfeed : true}
              onChange={(dsp) => update({ dsp })}
              settings={draft.dsp}
              title={
                isThisPhone
                  ? "DSP engine (this phone)"
                  : "DSP engine (server, 64-bit)"
              }
              unavailable={
                dspSupport && !dspSupport.available ? dspSupport.reason : null
              }
            />
          </Section>
        ) : null}

        {supports?.exclusive ||
        supports?.max_format ||
        supports?.dsd ||
        volumeModes.length > 1 ? (
          <Section title="Device setup">
            {supports?.exclusive ? (
              <View style={styles.switchRow}>
                <View style={styles.switchText}>
                  <Text style={styles.rowTitle}>Exclusive mode</Text>
                  <Text style={styles.note}>
                    Bit-perfect: the device runs at each file&apos;s own rate
                    and depth, and other apps wait.
                  </Text>
                </View>
                <Switch
                  accessibilityLabel="Exclusive mode"
                  onValueChange={(exclusive) => update({ exclusive })}
                  value={draft.exclusive}
                />
              </View>
            ) : null}
            {volumeModes.length > 1 ? (
              <>
                <Text style={[styles.label, styles.spaced]}>
                  Volume control
                </Text>
                <Chips
                  onSelect={(value) =>
                    update({ volume_mode: value as ZoneVolumeMode })
                  }
                  options={volumeModes.map((mode) => ({
                    value: mode,
                    label: volumeModeLabel(mode).split(" (")[0]!,
                  }))}
                  selected={draft.volume_mode}
                />
                <Text style={styles.note}>
                  {volumeModeLabel(draft.volume_mode)}
                </Text>
              </>
            ) : null}
            {supports?.max_format ? (
              <>
                <Text style={[styles.label, styles.spaced]}>
                  Max sample rate
                </Text>
                <Chips
                  onSelect={(value) =>
                    update({
                      max_sample_rate: value === "none" ? null : Number(value),
                    })
                  }
                  options={[
                    { value: "none", label: "Device max" },
                    ...SAMPLE_RATE_CHOICES.map((rate) => ({
                      value: String(rate),
                      label: formatSampleRate(rate),
                    })),
                  ]}
                  selected={
                    draft.max_sample_rate === null
                      ? "none"
                      : String(draft.max_sample_rate)
                  }
                />
              </>
            ) : null}
            {supports?.dsd ? (
              <>
                <Text style={[styles.label, styles.spaced]}>DSD playback</Text>
                <Chips
                  onSelect={(value) =>
                    update({ dsd_strategy: value as DsdStrategy })
                  }
                  options={DSD_STRATEGIES.map((strategy) => ({
                    value: strategy.value,
                    label: strategy.label,
                  }))}
                  selected={draft.dsd_strategy}
                />
                <Text style={styles.note}>
                  {
                    DSD_STRATEGIES.find(
                      (strategy) => strategy.value === draft.dsd_strategy,
                    )?.description
                  }
                </Text>
                {draft.dsd_strategy !== "pcm" ? (
                  <>
                    <Text style={[styles.label, styles.spaced]}>
                      Max DSD rate
                    </Text>
                    <Chips
                      onSelect={(value) =>
                        update({
                          max_dsd_rate: value === "none" ? null : Number(value),
                        })
                      }
                      options={[
                        { value: "none", label: "Device max" },
                        ...DSD_RATES.map((rate) => ({
                          value: String(rate),
                          label: dsdRateLabel(rate),
                        })),
                      ]}
                      selected={
                        draft.max_dsd_rate === null
                          ? "none"
                          : String(draft.max_dsd_rate)
                      }
                    />
                  </>
                ) : null}
              </>
            ) : null}
          </Section>
        ) : null}

        {zone.output ? (
          <Section title="Signal path">
            <Text style={styles.note}>{zone.output.note}</Text>
            {stream ? <SignalPathStages stream={stream} /> : null}
          </Section>
        ) : null}

        {supports?.private ? (
          <Section title="Privacy">
            <View style={styles.switchRow}>
              <View style={styles.switchText}>
                <View style={styles.inlineStart}>
                  <Lock color="white" size={14} />
                  <Text style={styles.rowTitle}>Private zone</Text>
                </View>
                <Text style={styles.note}>
                  {isThisPhone
                    ? "Only this phone sees and controls this zone, and it stays out of groups."
                    : "Set this on the device the zone runs on."}
                </Text>
              </View>
              <Switch
                accessibilityLabel="Private zone"
                disabled={!isThisPhone}
                onValueChange={(value) => update({ private: value })}
                value={draft.private}
              />
            </View>
          </Section>
        ) : null}

        {(zone.settings?.notes ?? []).map((note) => (
          <Text key={note} style={styles.note}>
            {note}
          </Text>
        ))}
      </ScrollView>
      {problem ? <Text style={styles.problem}>{problem}</Text> : null}
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ disabled: !changed || Boolean(problem) }}
        disabled={!changed || Boolean(problem) || saving}
        onPress={save}
        style={[
          styles.primary,
          (!changed || Boolean(problem) || saving) && styles.disabled,
        ]}
      >
        <Text style={styles.primaryText}>{saving ? "Saving…" : "Save"}</Text>
      </Pressable>
    </View>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <View style={styles.section}>
      <Text style={styles.sectionTitle}>{title}</Text>
      {children}
    </View>
  );
}

function Chips({
  options,
  selected,
  onSelect,
}: {
  options: { value: string; label: string }[];
  selected: string;
  onSelect: (value: string) => void;
}) {
  return (
    <View style={styles.chips}>
      {options.map((option) => {
        const active = option.value === selected;
        return (
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ selected: active }}
            key={option.value}
            onPress={() => onSelect(option.value)}
            style={[styles.chip, active && styles.choiceActive]}
          >
            <Text style={[styles.chipText, active && styles.chipTextActive]}>
              {option.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function Stepper({
  label,
  value,
  text,
  range,
  step = 1,
  onChange,
}: {
  label: string;
  value: number;
  text: string;
  range: { min: number; max: number };
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <View style={styles.stepper}>
      <Text style={styles.stepperLabel}>{label}</Text>
      <Pressable
        accessibilityLabel={`Lower ${label}`}
        accessibilityRole="button"
        hitSlop={8}
        onPress={() =>
          onChange(steppedValue(value, -step, range.min, range.max))
        }
        style={styles.stepButton}
      >
        <Minus color="white" size={16} />
      </Pressable>
      <Text style={styles.stepperValue}>{text}</Text>
      <Pressable
        accessibilityLabel={`Raise ${label}`}
        accessibilityRole="button"
        hitSlop={8}
        onPress={() =>
          onChange(steppedValue(value, step, range.min, range.max))
        }
        style={styles.stepButton}
      >
        <Plus color="white" size={16} />
      </Pressable>
    </View>
  );
}

/** Roon's quality-light colours: purple, blue, green, yellow. */
const QUALITY_COLORS: Record<ZoneStreamQuality, string> = {
  lossless: "#e6a27a",
  enhanced: "#8fc3e0",
  high: "#a3c98b",
  low: "#e6c599",
};

/** The quality light, then source, each step in order, and the output. */
function SignalPathStages({ stream }: { stream: ZoneStreamInfo }) {
  const quality = streamQuality(stream);
  const text = streamQualityText(quality);
  const color = QUALITY_COLORS[quality];
  const stages = [
    { kicker: "Source", label: stream.source },
    ...stream.steps.map((label) => ({ kicker: null, label })),
    { kicker: "Output", label: stream.output },
  ];
  return (
    <View style={styles.spaced}>
      <View style={[styles.qualityCard, { borderColor: `${color}55` }]}>
        <View
          style={[
            styles.qualityLight,
            { backgroundColor: color, shadowColor: color },
          ]}
        />
        <View style={styles.switchText}>
          <Text style={[styles.rowTitle, { color }]}>
            {stream.bit_perfect ? `${text.label} · Bit-perfect` : text.label}
          </Text>
          <Text style={styles.note}>
            {stream.bit_perfect
              ? "Every sample of the file reaches the output untouched."
              : text.description}
          </Text>
        </View>
      </View>
      {stages.map((stage, index) => (
        <View key={`${index}-${stage.label}`} style={styles.stage}>
          <View
            style={[
              styles.stageDot,
              stage.kicker ? { borderColor: color } : null,
            ]}
          />
          <View style={styles.switchText}>
            {stage.kicker ? (
              <Text style={styles.stageKicker}>{stage.kicker}</Text>
            ) : null}
            <Text style={stage.kicker ? styles.rowTitle : styles.note}>
              {stage.label}
            </Text>
          </View>
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  qualityCard: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
    borderWidth: 1,
    borderRadius: 12,
    padding: 12,
    marginBottom: 8,
    backgroundColor: "rgba(255,255,255,0.03)",
  },
  qualityLight: {
    width: 9,
    height: 9,
    borderRadius: 5,
    marginTop: 5,
    shadowOpacity: 0.8,
    shadowRadius: 6,
    shadowOffset: { width: 0, height: 0 },
  },
  stage: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
    paddingVertical: 4,
  },
  stageDot: {
    width: 10,
    height: 10,
    borderRadius: 5,
    borderWidth: 2,
    borderColor: "rgba(255,255,255,0.2)",
    marginTop: 4,
  },
  stageKicker: {
    color: "#8e8a84",
    fontSize: 10,
    fontWeight: "600",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  sheet: { flex: 1, backgroundColor: palette.background, paddingTop: 16 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 20,
    gap: 12,
  },
  headerText: { flex: 1 },
  title: { color: "white", fontSize: 22, fontWeight: "700" },
  subtitle: { color: palette.secondary, fontSize: 13, marginTop: 2 },
  content: { padding: 20, gap: 22, paddingBottom: 32 },
  section: { gap: 8 },
  sectionTitle: {
    color: palette.muted,
    fontSize: 12,
    fontWeight: "700",
    letterSpacing: 0.6,
    textTransform: "uppercase",
  },
  label: { color: palette.secondary, fontSize: 13 },
  spaced: { marginTop: 8 },
  input: {
    borderColor: palette.borderStrong,
    borderRadius: 10,
    borderWidth: 1,
    color: "white",
    fontSize: 16,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  inline: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  inlineStart: { flexDirection: "row", alignItems: "center", gap: 6 },
  note: { color: palette.muted, fontSize: 12, lineHeight: 16 },
  link: { color: palette.secondary, fontSize: 12, fontWeight: "600" },
  icons: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  iconChoice: {
    alignItems: "center",
    borderColor: palette.border,
    borderRadius: 10,
    borderWidth: 1,
    height: 40,
    justifyContent: "center",
    width: 40,
  },
  choiceActive: { backgroundColor: "white", borderColor: "white" },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  chip: {
    borderColor: palette.borderStrong,
    borderRadius: 999,
    borderWidth: 1,
    paddingHorizontal: 12,
    paddingVertical: 7,
  },
  chipText: { color: "white", fontSize: 13, fontWeight: "600" },
  chipTextActive: { color: "black" },
  switchRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  switchText: { flex: 1, gap: 2 },
  rowTitle: { color: "white", fontSize: 15, fontWeight: "600" },
  stepper: { flexDirection: "row", alignItems: "center", gap: 10 },
  stepperLabel: { color: palette.secondary, fontSize: 13, flex: 1 },
  stepperValue: {
    color: "white",
    fontSize: 14,
    fontVariant: ["tabular-nums"],
    minWidth: 76,
    textAlign: "center",
  },
  stepButton: {
    alignItems: "center",
    borderColor: palette.borderStrong,
    borderRadius: 999,
    borderWidth: 1,
    height: 32,
    justifyContent: "center",
    width: 32,
  },
  problem: {
    color: "#daab6c",
    fontSize: 12,
    paddingHorizontal: 20,
    paddingBottom: 8,
  },
  primary: {
    alignItems: "center",
    backgroundColor: "white",
    borderRadius: 999,
    marginHorizontal: 20,
    paddingVertical: 13,
  },
  primaryText: { color: "black", fontSize: 15, fontWeight: "700" },
  disabled: { opacity: 0.4 },
});
