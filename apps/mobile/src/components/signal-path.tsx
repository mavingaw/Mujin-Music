import {
  activeEqBands,
  dspMayClip,
  eqPresets,
  recommendedHeadroomDb,
  SIGNAL_CLIPPING_COLOR,
  signalLightColor,
  signalLightLabel,
  signalPathSummary,
  signalQualityColors,
  signalQualityLabels,
  type DspSettings,
  type SignalPath,
  type SignalQuality,
  type SignalStage,
  type SignalStageKind,
  type VolumeLevelingMode,
} from "@parson/music-sdk";
import {
  ArrowLeftRight,
  AudioWaveform,
  Cpu,
  FileAudio,
  type LucideIcon,
  Server,
  Sparkles,
  Speaker,
  Volume2,
} from "lucide-react-native";
import { useState } from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
} from "react-native";
import Svg, { Circle, Path } from "react-native-svg";

import { palette } from "@/constants/colors";
import { volumeLevelingOptions } from "@/lib/signal-path";
import { immediatePressFeedback } from "@/lib/press-feedback";

const STAGE_ICONS: Record<SignalStageKind, LucideIcon> = {
  source: FileAudio,
  dsp: Sparkles,
  conversion: ArrowLeftRight,
  encode: AudioWaveform,
  transport: Server,
  decode: Cpu,
  volume: Volume2,
  output: Speaker,
};

const LEGEND: SignalQuality[] = [
  "lossless",
  "enhanced",
  "high_quality",
  "low_quality",
];

/** Roon's light: a sparkle for lossless and enhanced, a dot otherwise. */
export function QualityGlyph({
  quality,
  size = 14,
  clipping = false,
}: {
  quality: SignalQuality | null;
  size?: number;
  /** The DSP chain is clipping: the light turns red. */
  clipping?: boolean;
}) {
  const color = clipping
    ? SIGNAL_CLIPPING_COLOR
    : quality
      ? signalQualityColors[quality]
      : palette.muted;
  const sparkle = quality === "lossless" || quality === "enhanced";
  return (
    <Svg height={size} viewBox="0 0 16 16" width={size}>
      {sparkle ? (
        <Path
          d="M8 0.8 9.5 6.5 15.2 8 9.5 9.5 8 15.2 6.5 9.5 0.8 8 6.5 6.5Z"
          fill={color}
        />
      ) : (
        <Circle cx={8} cy={8} fill={color} r={3.6} />
      )}
    </Svg>
  );
}

/** The quality light as a pill under the now-playing title. */
export function SignalPathBadge({
  onPress,
  path,
}: {
  onPress: () => void;
  path: SignalPath | null;
}) {
  const quality = path?.quality ?? null;
  const label = path
    ? `Signal path: ${signalLightLabel(path)}${path.highRes ? ", high resolution" : ""}`
    : "Signal path";
  return (
    <Pressable
      {...immediatePressFeedback}
      accessibilityHint="Shows each stage between the file and the speaker"
      accessibilityLabel={label}
      accessibilityRole="button"
      hitSlop={8}
      onPress={onPress}
      style={styles.badge}
    >
      <QualityGlyph clipping={path?.clipping} quality={quality} size={13} />
      {path ? (
        <>
          <Text
            style={[styles.badgeQuality, { color: signalLightColor(path) }]}
          >
            {signalLightLabel(path)}
          </Text>
          {path.highRes ? <HiRes /> : null}
          <Text numberOfLines={1} style={styles.badgeFormat}>
            {path.source?.summary ?? "Format unknown"}
          </Text>
        </>
      ) : (
        <Text style={styles.badgeFormat}>Signal path</Text>
      )}
    </Pressable>
  );
}

function HiRes() {
  return (
    <View style={styles.hiRes}>
      <Text style={styles.hiResText}>HI-RES</Text>
    </View>
  );
}

/** Drawer body: header, stages, leveling and legend. */
export function SignalPathContent({
  casting,
  dsp,
  levelingMode,
  loading,
  onDspChange,
  onLevelingModeChange,
  path,
}: {
  casting: boolean;
  /** `null` where the device has no effects graph to run DSP in. */
  dsp?: DspSettings | null;
  levelingMode: VolumeLevelingMode;
  loading: boolean;
  onDspChange?: (settings: DspSettings) => void;
  onLevelingModeChange: (mode: VolumeLevelingMode) => void;
  path: SignalPath | null;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <View style={styles.content}>
      <Text accessibilityRole="header" style={styles.heading}>
        Signal path:{" "}
        <Text
          style={{
            color: path ? signalLightColor(path) : palette.secondary,
          }}
        >
          {path ? signalLightLabel(path) : loading ? "Checking" : "Unknown"}
        </Text>
      </Text>
      {path ? (
        <Text style={styles.subtitle}>
          {signalPathSummary(path)}. Tap a stage to learn more.
        </Text>
      ) : null}
      {path?.bitPerfectToOutput && path.quality !== "lossless" ? (
        <View style={styles.bitPerfect}>
          <QualityGlyph quality="lossless" size={10} />
          <Text
            style={[
              styles.bitPerfectText,
              { color: signalQualityColors.lossless },
            ]}
          >
            Bit-perfect up to the {casting ? "Cast device" : "system mixer"}
          </Text>
        </View>
      ) : null}
      <ScrollView style={styles.stages}>
        {path?.stages.map((stage, index) => (
          <StageRow
            expanded={selected === stage.id}
            key={stage.id}
            last={index === path.stages.length - 1}
            onPress={() =>
              setSelected((current) => (current === stage.id ? null : stage.id))
            }
            stage={stage}
          />
        ))}
        {path && !path.serverKnown ? (
          <Text style={styles.note}>
            The server&apos;s part of the path could not be loaded, so the
            source is described from the library.
          </Text>
        ) : null}
        {path ? <Text style={styles.note}>{path.description}</Text> : null}
      </ScrollView>
      {!casting && dsp && onDspChange ? (
        <DspControls onChange={onDspChange} settings={dsp} />
      ) : null}
      {!casting ? (
        <View style={styles.leveling}>
          <Text style={styles.levelingLabel}>Volume leveling</Text>
          <View accessibilityRole="radiogroup" style={styles.segments}>
            {volumeLevelingOptions.map((option) => {
              const active = option.mode === levelingMode;
              return (
                <Pressable
                  accessibilityLabel={`Volume leveling ${option.label}`}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: active }}
                  key={option.mode}
                  onPress={() => onLevelingModeChange(option.mode)}
                  style={[styles.segment, active && styles.segmentActive]}
                >
                  <Text
                    style={[
                      styles.segmentText,
                      active && styles.segmentTextActive,
                    ]}
                  >
                    {option.label}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>
      ) : null}
      <View style={styles.legend}>
        {LEGEND.map((quality) => (
          <View key={quality} style={styles.legendItem}>
            <QualityGlyph quality={quality} size={10} />
            <Text style={styles.legendText}>
              {signalQualityLabels[quality]}
            </Text>
          </View>
        ))}
      </View>
    </View>
  );
}

const MOBILE_EQ_PRESETS = eqPresets.filter((preset) =>
  ["flat", "bass", "treble", "vocal", "loudness"].includes(preset.id),
);

/**
 * DSP engine controls: headroom and parametric EQ presets, plus crossfeed
 * where the chain can run it. Used for the phone's own chain and for a
 * server output's (which the server runs in 64-bit float).
 */
export function DspControls({
  onChange,
  settings,
  title = "DSP engine",
  unavailable = null,
  crossfeed = null,
  filters = null,
}: {
  onChange: (settings: DspSettings) => void;
  settings: DspSettings;
  title?: string;
  /** Why the chain cannot run here; every control is greyed. */
  unavailable?: string | null;
  /** `true`/`false` shows a crossfeed switch; `null` hides it. */
  crossfeed?: boolean | null;
  /**
   * Convolution filters on the server (a server output convolves; this
   * phone's engine cannot); `null` hides convolution.
   */
  filters?: { id: string; name: string; peakGainDb: number }[] | null;
}) {
  if (unavailable) {
    return (
      <View style={[styles.dsp, styles.dspOff]}>
        <View style={styles.dspRow}>
          <Text style={styles.levelingLabel}>{title}</Text>
          <Switch accessibilityLabel={title} disabled value={false} />
        </View>
        <Text style={styles.dspLabel}>{unavailable}</Text>
      </View>
    );
  }
  const clipping = dspMayClip(settings);
  const needed = recommendedHeadroomDb(settings);
  const bands = activeEqBands({
    ...settings,
    enabled: true,
    eq: { ...settings.eq, enabled: true },
  }).length;
  return (
    <View style={styles.dsp}>
      <View style={styles.dspRow}>
        <Text style={styles.levelingLabel}>{title}</Text>
        <Switch
          accessibilityLabel={title}
          onValueChange={(enabled) => onChange({ ...settings, enabled })}
          trackColor={{ true: signalQualityColors.enhanced }}
          value={settings.enabled}
        />
      </View>
      <View style={[styles.dspRow, !settings.enabled && styles.dspOff]}>
        <Text style={styles.dspLabel}>
          Headroom {settings.headroom.gainDb.toFixed(1)} dB
          {clipping ? `  ·  EQ needs ${needed.toFixed(1)} dB` : ""}
        </Text>
        <Switch
          accessibilityLabel="Headroom adjustment"
          disabled={!settings.enabled}
          onValueChange={(enabled) =>
            onChange({
              ...settings,
              headroom: {
                enabled,
                gainDb: enabled && clipping ? needed : settings.headroom.gainDb,
              },
            })
          }
          trackColor={{ true: signalQualityColors.enhanced }}
          value={settings.headroom.enabled}
        />
      </View>
      <View style={[styles.dspRow, !settings.enabled && styles.dspOff]}>
        <Text style={styles.dspLabel}>
          Parametric EQ, {bands} {bands === 1 ? "band" : "bands"}
        </Text>
        <Switch
          accessibilityLabel="Parametric EQ"
          disabled={!settings.enabled}
          onValueChange={(enabled) =>
            onChange({ ...settings, eq: { ...settings.eq, enabled } })
          }
          trackColor={{ true: signalQualityColors.enhanced }}
          value={settings.eq.enabled}
        />
      </View>
      <View
        accessibilityRole="radiogroup"
        style={[
          styles.presetRow,
          (!settings.enabled || !settings.eq.enabled) && styles.dspOff,
        ]}
      >
        {MOBILE_EQ_PRESETS.map((preset) => {
          const active = settings.eq.preset === preset.id;
          return (
            <Pressable
              accessibilityLabel={`EQ ${preset.label}`}
              accessibilityRole="radio"
              accessibilityState={{ checked: active }}
              disabled={!settings.enabled || !settings.eq.enabled}
              key={preset.id}
              onPress={() =>
                onChange({
                  ...settings,
                  eq: {
                    ...settings.eq,
                    preset: preset.id,
                    bands: preset.bands.map((band) => ({ ...band })),
                  },
                })
              }
              style={[styles.segment, active && styles.segmentActive]}
            >
              <Text
                style={[styles.segmentText, active && styles.segmentTextActive]}
              >
                {preset.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      {crossfeed !== null ? (
        <View
          style={[
            styles.dspRow,
            (!settings.enabled || !crossfeed) && styles.dspOff,
          ]}
        >
          <Text style={styles.dspLabel}>
            {crossfeed
              ? `Crossfeed, ${settings.crossfeed.strength}`
              : "Crossfeed needs a stereo device"}
          </Text>
          <Switch
            accessibilityLabel="Crossfeed"
            disabled={!settings.enabled || !crossfeed}
            onValueChange={(enabled) =>
              onChange({
                ...settings,
                crossfeed: { ...settings.crossfeed, enabled },
              })
            }
            trackColor={{ true: signalQualityColors.enhanced }}
            value={crossfeed && settings.crossfeed.enabled}
          />
        </View>
      ) : null}
      {filters ? (
        <ConvolutionRow
          filters={filters}
          onChange={onChange}
          settings={settings}
        />
      ) : null}
    </View>
  );
}

function ConvolutionRow({
  filters,
  onChange,
  settings,
}: {
  filters: { id: string; name: string; peakGainDb: number }[];
  onChange: (settings: DspSettings) => void;
  settings: DspSettings;
}) {
  const convolution = settings.convolution ?? {
    enabled: false,
    filterId: null,
  };
  const chosen = filters.find((filter) => filter.id === convolution.filterId);
  return (
    <>
      <View
        style={[
          styles.dspRow,
          (!settings.enabled || !filters.length) && styles.dspOff,
        ]}
      >
        <Text style={styles.dspLabel}>
          {filters.length
            ? `Convolution${chosen ? `, ${chosen.name}` : ""}`
            : "Convolution: upload a filter on the web"}
        </Text>
        <Switch
          accessibilityLabel="Convolution"
          disabled={!settings.enabled || !filters.length}
          onValueChange={(enabled) =>
            onChange({
              ...settings,
              convolution: {
                enabled,
                filterId: convolution.filterId ?? filters[0]?.id ?? null,
              },
            })
          }
          trackColor={{ true: signalQualityColors.enhanced }}
          value={convolution.enabled && Boolean(chosen)}
        />
      </View>
      {filters.length > 1 ? (
        <View
          accessibilityRole="radiogroup"
          style={[
            styles.presetRow,
            (!settings.enabled || !convolution.enabled) && styles.dspOff,
          ]}
        >
          {filters.map((filter) => {
            const active = filter.id === convolution.filterId;
            return (
              <Pressable
                accessibilityLabel={`Convolution filter ${filter.name}`}
                accessibilityRole="radio"
                accessibilityState={{ checked: active }}
                disabled={!settings.enabled}
                key={filter.id}
                onPress={() =>
                  onChange({
                    ...settings,
                    convolution: { enabled: true, filterId: filter.id },
                  })
                }
                style={[styles.segment, active && styles.segmentActive]}
              >
                <Text
                  style={[
                    styles.segmentText,
                    active && styles.segmentTextActive,
                  ]}
                >
                  {filter.name}
                </Text>
              </Pressable>
            );
          })}
        </View>
      ) : null}
    </>
  );
}

function StageRow({
  expanded,
  last,
  onPress,
  stage,
}: {
  expanded: boolean;
  last: boolean;
  onPress: () => void;
  stage: SignalStage;
}) {
  const Icon = STAGE_ICONS[stage.kind];
  const color = stage.quality
    ? signalQualityColors[stage.quality]
    : palette.secondary;
  return (
    <Pressable
      accessibilityHint="Shows what this stage does"
      accessibilityLabel={`${stage.title}, ${stage.detail}, ${stage.quality ? signalQualityLabels[stage.quality] : "unknown quality"}`}
      accessibilityRole="button"
      accessibilityState={{ expanded }}
      onPress={onPress}
      style={styles.stage}
    >
      <View style={styles.rail}>
        <View style={styles.iconRing}>
          <Icon color="#d4d4d8" size={16} />
        </View>
        {!last ? <View style={styles.railLine} /> : null}
      </View>
      <View style={styles.stageText}>
        <View style={styles.stageTitleRow}>
          <Text numberOfLines={1} style={styles.stageTitle}>
            {stage.title}
          </Text>
          <QualityGlyph quality={stage.quality} size={11} />
        </View>
        <Text style={[styles.stageDetail, { color }]}>{stage.detail}</Text>
        {expanded ? (
          <Text style={styles.stageExplanation}>{stage.explanation}</Text>
        ) : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  badge: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
    marginTop: 12,
    minHeight: 32,
    paddingLeft: 10,
    paddingRight: 12,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: palette.borderStrong,
    backgroundColor: "rgba(255,255,255,0.05)",
  },
  badgeQuality: { fontSize: 13, fontWeight: "700" },
  badgeFormat: { flexShrink: 1, color: palette.secondary, fontSize: 13 },
  hiRes: {
    borderWidth: 1,
    borderColor: "#c9a2ff",
    borderRadius: 4,
    paddingHorizontal: 3,
  },
  hiResText: {
    color: "#c9a2ff",
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.5,
  },
  content: { paddingHorizontal: 20, paddingBottom: 4 },
  heading: { color: "white", fontSize: 19, fontWeight: "800" },
  subtitle: { color: palette.secondary, fontSize: 13, marginTop: 3 },
  bitPerfect: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    marginTop: 6,
  },
  bitPerfectText: { fontSize: 13, fontWeight: "600" },
  stages: { maxHeight: 380, marginTop: 12 },
  stage: { flexDirection: "row", gap: 12, minHeight: 56 },
  rail: { alignItems: "center", width: 34 },
  iconRing: {
    width: 34,
    height: 34,
    borderRadius: 17,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.16)",
    backgroundColor: palette.elevatedStrong,
    alignItems: "center",
    justifyContent: "center",
  },
  railLine: {
    flex: 1,
    width: StyleSheet.hairlineWidth * 2,
    backgroundColor: "rgba(255,255,255,0.14)",
  },
  stageText: { flex: 1, paddingTop: 2, paddingBottom: 12 },
  stageTitleRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  stageTitle: { color: "white", fontSize: 15, fontWeight: "700" },
  stageDetail: { fontSize: 14, marginTop: 2 },
  stageExplanation: {
    color: palette.secondary,
    fontSize: 13,
    lineHeight: 19,
    marginTop: 6,
  },
  note: {
    color: palette.muted,
    fontSize: 12,
    lineHeight: 17,
    marginBottom: 8,
  },
  leveling: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    marginTop: 8,
    paddingTop: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: palette.border,
  },
  levelingLabel: { color: "white", fontSize: 14, fontWeight: "600" },
  segments: {
    flexDirection: "row",
    borderRadius: 999,
    backgroundColor: "rgba(255,255,255,0.08)",
    padding: 2,
  },
  segment: {
    minHeight: 30,
    paddingHorizontal: 10,
    borderRadius: 999,
    justifyContent: "center",
  },
  segmentActive: { backgroundColor: "white" },
  segmentText: { color: palette.secondary, fontSize: 12, fontWeight: "600" },
  segmentTextActive: { color: "black" },
  dsp: {
    gap: 6,
    marginTop: 8,
    paddingTop: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: palette.border,
  },
  dspRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  dspLabel: { color: palette.secondary, fontSize: 13, flexShrink: 1 },
  dspOff: { opacity: 0.45 },
  presetRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 4,
  },
  legend: {
    flexDirection: "row",
    flexWrap: "wrap",
    columnGap: 14,
    rowGap: 4,
    marginTop: 12,
  },
  legendItem: { flexDirection: "row", alignItems: "center", gap: 5 },
  legendText: { color: palette.muted, fontSize: 11 },
});
