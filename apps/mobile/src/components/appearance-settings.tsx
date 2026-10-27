import {
  type Appearance,
  type ThemePreset,
  CORNER_RADIUS_RANGE,
  HOME_ROW_LABELS,
  hexFromArgb,
  moveHomeRow,
  NOW_PLAYING_LAYOUT_LABELS,
  NOW_PLAYING_LAYOUTS,
  normalizeAppearance,
  PRESET_DESCRIPTIONS,
  PRESET_LABELS,
  THEME_PRESETS,
} from "@parson/music-sdk";
import { useMemo, useState } from "react";
import {
  Alert,
  Appearance as SystemAppearance,
  Platform,
  Pressable,
  StyleSheet,
  Switch,
  Text,
  View,
} from "react-native";
import { ArrowDown, ArrowUp, Check, Eye, EyeOff } from "lucide-react-native";

import {
  activeTheme,
  fonts,
  palette,
  storedAppearance,
  systemAccent,
} from "@/constants/colors";
import {
  type MobilePalette,
  mobilePalette,
  mobileRadius,
  resolveMobileTheme,
} from "@/lib/appearance";
import { usePlayer } from "@/providers/player-provider";
import {
  applyAppearance,
  currentStoredAppearance,
} from "@/providers/appearance-sync";

const SWATCHES: (string | null)[] = [
  null,
  "#c9a24a",
  "#8fb07a",
  "#4fa7a0",
  "#6c9fd0",
  "#7d83e8",
  "#c27ac0",
  "#e07a8a",
];

function themeFor(draft: Appearance) {
  return resolveMobileTheme(
    { ...currentStoredAppearance(), appearance: draft },
    {
      systemDark: SystemAppearance.getColorScheme() !== "light",
      systemAccent,
    },
  );
}

/** A small Home screen drawn in a theme's colours, for live preview. */
function Preview({
  colors,
  radius,
}: {
  colors: MobilePalette;
  radius: number;
}) {
  return (
    <View
      accessibilityLabel="Theme preview"
      style={[
        styles.preview,
        { backgroundColor: colors.background, borderRadius: radius },
      ]}
    >
      <Text style={[styles.previewEyebrow, { color: colors.tertiary }]}>
        GOOD EVENING
      </Text>
      <Text style={[styles.previewTitle, { color: colors.text }]}>
        Your library
      </Text>
      <View style={styles.previewRow}>
        {[0, 1, 2].map((tile) => (
          <View
            key={tile}
            style={[
              styles.previewTile,
              {
                backgroundColor:
                  tile === 0 ? colors.accent : colors.surfaceStrong,
                borderRadius: Math.max(2, radius / 3),
              },
            ]}
          />
        ))}
      </View>
      <View
        style={[
          styles.previewPlayer,
          { backgroundColor: colors.elevated, borderRadius: radius / 1.5 },
        ]}
      >
        <View style={{ flex: 1 }}>
          <Text style={{ color: colors.text, fontSize: 13, fontWeight: "600" }}>
            The Slip
          </Text>
          <Text style={{ color: colors.secondary, fontSize: 11 }}>
            Nine Inch Nails · 24/96 FLAC
          </Text>
        </View>
        <View style={[styles.previewPlay, { backgroundColor: colors.accent }]}>
          <Text style={{ color: colors.accentText, fontWeight: "700" }}>▶</Text>
        </View>
      </View>
    </View>
  );
}

function Option({
  active,
  label,
  onPress,
}: {
  active: boolean;
  label: string;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="radio"
      accessibilityState={{ selected: active }}
      onPress={onPress}
      style={[styles.chip, active && styles.chipActive]}
    >
      <Text style={[styles.chipText, active && styles.chipTextActive]}>
        {label}
      </Text>
    </Pressable>
  );
}

export function AppearanceSettings() {
  const [draft, setDraft] = useState<Appearance>(storedAppearance.appearance);
  const [saving, setSaving] = useState(false);
  const { isPlaying } = usePlayer();
  const update = (patch: Partial<Appearance>) =>
    setDraft((value) => normalizeAppearance({ ...value, ...patch }));
  const theme = useMemo(() => themeFor(draft), [draft]);
  const colors = useMemo(() => mobilePalette(theme), [theme]);
  const presetColors = useMemo(
    () =>
      THEME_PRESETS.map((preset) => ({
        preset,
        colors: mobilePalette(themeFor({ ...draft, preset })),
      })),
    [draft],
  );
  const changed =
    JSON.stringify(normalizeAppearance(draft)) !==
    JSON.stringify(normalizeAppearance(storedAppearance.appearance));
  const androidDynamic = Platform.OS === "android" && systemAccent !== null;

  const apply = () => {
    const run = async () => {
      setSaving(true);
      try {
        await applyAppearance(draft);
      } finally {
        setSaving(false);
      }
    };
    if (isPlaying) {
      Alert.alert(
        "Apply the new look?",
        "Mujin Music redraws every screen, which pauses what is playing for a moment.",
        [
          { text: "Not now", style: "cancel" },
          { text: "Apply", onPress: () => void run() },
        ],
      );
    } else {
      void run();
    }
  };

  return (
    <View>
      <Text style={styles.heading}>Appearance</Text>
      <Text style={styles.hint}>
        Saved to this profile and shared with Mujin Music on the web and the
        desktop.
      </Text>
      <Preview colors={colors} radius={mobileRadius(theme)} />

      <Text style={styles.label}>Theme</Text>
      {presetColors.map(({ preset, colors: swatch }) => {
        const active = draft.preset === preset;
        return (
          <Pressable
            accessibilityRole="radio"
            accessibilityState={{ selected: active }}
            key={preset}
            onPress={() =>
              update(
                preset === "mujin-light"
                  ? { preset, color_scheme: "light" }
                  : preset === "material-you" && !draft.accent
                    ? { preset, accent_from_artwork: !androidDynamic }
                    : { preset },
              )
            }
            style={[styles.preset, active && styles.presetActive]}
          >
            <View
              style={[
                styles.presetSwatch,
                { backgroundColor: swatch.background },
              ]}
            >
              <View
                style={[styles.presetDot, { backgroundColor: swatch.accent }]}
              />
              <View
                style={[styles.presetLine, { backgroundColor: swatch.text }]}
              />
            </View>
            <View style={{ flex: 1 }}>
              <Text style={styles.presetName}>
                {PRESET_LABELS[preset as ThemePreset]}
              </Text>
              <Text style={styles.presetText} numberOfLines={2}>
                {PRESET_DESCRIPTIONS[preset as ThemePreset]}
              </Text>
            </View>
            {active ? <Check color={palette.accent} size={18} /> : null}
          </Pressable>
        );
      })}

      <Text style={styles.label}>Light or dark</Text>
      <View style={styles.row}>
        {(["system", "dark", "light"] as const).map((scheme) => (
          <Option
            active={draft.color_scheme === scheme}
            key={scheme}
            label={
              scheme === "system"
                ? "Match phone"
                : scheme === "dark"
                  ? "Dark"
                  : "Light"
            }
            onPress={() =>
              update({
                color_scheme: scheme,
                ...(draft.preset === "mujin-light" && scheme === "dark"
                  ? { preset: "mujin-dark" as const }
                  : draft.preset === "mujin-dark" && scheme === "light"
                    ? { preset: "mujin-light" as const }
                    : {}),
              })
            }
          />
        ))}
      </View>

      <Text style={styles.label}>Accent</Text>
      <View style={styles.row}>
        {SWATCHES.map((value) => {
          const active = !draft.accent_from_artwork && draft.accent === value;
          return (
            <Pressable
              accessibilityLabel={value ?? "Mujin Copper"}
              accessibilityRole="radio"
              accessibilityState={{ selected: active }}
              key={value ?? "copper"}
              onPress={() =>
                update({ accent: value, accent_from_artwork: false })
              }
              style={[
                styles.swatch,
                { backgroundColor: value ?? "#d98b5f" },
                active && styles.swatchActive,
              ]}
            />
          );
        })}
      </View>
      <View style={styles.switchRow}>
        <View style={{ flex: 1 }}>
          <Text style={styles.switchLabel}>Colour from album art</Text>
          <Text style={styles.hint}>
            The accent (and Material You’s whole scheme) follows the playing
            cover.
          </Text>
        </View>
        <Switch
          onValueChange={(accent_from_artwork) =>
            update({ accent_from_artwork })
          }
          thumbColor={palette.text}
          trackColor={{ false: palette.washStrong, true: palette.accent }}
          value={draft.accent_from_artwork}
        />
      </View>
      {draft.preset === "material-you" && Platform.OS === "android" ? (
        <View style={styles.switchRow}>
          <View style={{ flex: 1 }}>
            <Text style={styles.switchLabel}>Use my wallpaper colours</Text>
            <Text style={styles.hint}>
              {androidDynamic
                ? `Android’s dynamic colour (${hexFromArgb(systemAccent!)}) seeds the scheme.`
                : "Needs Android 12 or later."}
            </Text>
          </View>
          <Switch
            disabled={!androidDynamic}
            onValueChange={(follow_system_colors) =>
              update({ follow_system_colors })
            }
            thumbColor={palette.text}
            trackColor={{ false: palette.washStrong, true: palette.accent }}
            value={draft.follow_system_colors && androidDynamic}
          />
        </View>
      ) : null}

      <Text style={styles.label}>Corner radius · {draft.corner_radius}px</Text>
      <View style={styles.row}>
        {[0, 4, 8, 12, 16, CORNER_RADIUS_RANGE.max].map((radius) => (
          <Option
            active={draft.corner_radius === radius}
            key={radius}
            label={radius === 8 ? "8 (default)" : String(radius)}
            onPress={() => update({ corner_radius: radius })}
          />
        ))}
      </View>

      <Text style={styles.label}>Density</Text>
      <View style={styles.row}>
        <Option
          active={draft.density === "comfortable"}
          label="Comfortable"
          onPress={() => update({ density: "comfortable" })}
        />
        <Option
          active={draft.density === "compact"}
          label="Compact"
          onPress={() => update({ density: "compact" })}
        />
      </View>

      <Text style={styles.label}>Now Playing</Text>
      <View style={styles.row}>
        {NOW_PLAYING_LAYOUTS.map((layout) => (
          <Option
            active={draft.now_playing_layout === layout}
            key={layout}
            label={NOW_PLAYING_LAYOUT_LABELS[layout]}
            onPress={() => update({ now_playing_layout: layout })}
          />
        ))}
      </View>

      <Text style={styles.label}>Home rows</Text>
      {draft.home_rows.map((row, index) => (
        <View key={row.id} style={styles.homeRow}>
          <Text
            style={[
              styles.homeRowText,
              !row.visible && { color: palette.tertiary },
            ]}
          >
            {HOME_ROW_LABELS[row.id]}
          </Text>
          <Pressable
            accessibilityLabel={`Move ${HOME_ROW_LABELS[row.id]} up`}
            disabled={index === 0}
            hitSlop={6}
            onPress={() =>
              update({ home_rows: moveHomeRow(draft.home_rows, row.id, -1) })
            }
            style={styles.iconButton}
          >
            <ArrowUp color={palette.secondary} size={16} />
          </Pressable>
          <Pressable
            accessibilityLabel={`Move ${HOME_ROW_LABELS[row.id]} down`}
            disabled={index === draft.home_rows.length - 1}
            hitSlop={6}
            onPress={() =>
              update({ home_rows: moveHomeRow(draft.home_rows, row.id, 1) })
            }
            style={styles.iconButton}
          >
            <ArrowDown color={palette.secondary} size={16} />
          </Pressable>
          <Pressable
            accessibilityLabel={`${row.visible ? "Hide" : "Show"} ${HOME_ROW_LABELS[row.id]}`}
            hitSlop={6}
            onPress={() =>
              update({
                home_rows: draft.home_rows.map((entry) =>
                  entry.id === row.id
                    ? { ...entry, visible: !entry.visible }
                    : entry,
                ),
              })
            }
            style={styles.iconButton}
          >
            {row.visible ? (
              <Eye color={palette.secondary} size={16} />
            ) : (
              <EyeOff color={palette.tertiary} size={16} />
            )}
          </Pressable>
        </View>
      ))}

      <Pressable
        accessibilityRole="button"
        disabled={!changed || saving}
        onPress={apply}
        style={[styles.apply, (!changed || saving) && { opacity: 0.45 }]}
      >
        <Text style={styles.applyText}>
          {saving ? "Applying…" : changed ? "Apply" : "Applied"}
        </Text>
      </Pressable>
      <Text style={styles.hint}>
        Showing {activeTheme.label}, {activeTheme.mode}. Text size follows your
        phone’s font size setting.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  heading: {
    color: palette.text,
    fontFamily: fonts.title,
    fontSize: 22,
    marginBottom: 4,
  },
  hint: { color: palette.tertiary, fontSize: 12, lineHeight: 17, marginTop: 4 },
  label: {
    color: palette.secondary,
    fontSize: 13,
    fontWeight: "600",
    marginBottom: 8,
    marginTop: 22,
  },
  preview: {
    borderColor: palette.border,
    borderWidth: StyleSheet.hairlineWidth,
    marginTop: 16,
    overflow: "hidden",
    padding: 16,
  },
  previewEyebrow: { fontSize: 10, fontWeight: "600", letterSpacing: 1.4 },
  previewTitle: { fontFamily: fonts.display, fontSize: 28, marginTop: 4 },
  previewRow: { flexDirection: "row", gap: 8, marginTop: 14 },
  previewTile: { aspectRatio: 1, flex: 1 },
  previewPlayer: {
    alignItems: "center",
    flexDirection: "row",
    marginTop: 14,
    padding: 10,
  },
  previewPlay: {
    alignItems: "center",
    borderRadius: 999,
    height: 34,
    justifyContent: "center",
    width: 34,
  },
  preset: {
    alignItems: "center",
    borderColor: palette.border,
    borderRadius: 14,
    borderWidth: 1,
    flexDirection: "row",
    gap: 12,
    marginBottom: 8,
    padding: 10,
  },
  presetActive: { borderColor: palette.accent },
  presetSwatch: {
    borderColor: palette.border,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    height: 46,
    justifyContent: "center",
    paddingHorizontal: 8,
    width: 56,
  },
  presetDot: { borderRadius: 999, height: 10, marginBottom: 6, width: 10 },
  presetLine: { borderRadius: 2, height: 4, opacity: 0.85, width: 30 },
  presetName: { color: palette.text, fontSize: 15, fontWeight: "600" },
  presetText: { color: palette.tertiary, fontSize: 12, lineHeight: 16 },
  row: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  chip: {
    borderColor: palette.borderStrong,
    borderRadius: 999,
    borderWidth: 1,
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  chipActive: { backgroundColor: palette.accent, borderColor: palette.accent },
  chipText: { color: palette.secondary, fontSize: 13, fontWeight: "600" },
  chipTextActive: { color: palette.accentText },
  swatch: { borderRadius: 999, height: 34, width: 34 },
  swatchActive: { borderColor: palette.text, borderWidth: 3 },
  switchRow: {
    alignItems: "center",
    flexDirection: "row",
    gap: 12,
    marginTop: 16,
  },
  switchLabel: { color: palette.text, fontSize: 15, fontWeight: "500" },
  homeRow: {
    alignItems: "center",
    borderBottomColor: palette.border,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    gap: 6,
    paddingVertical: 10,
  },
  homeRowText: { color: palette.text, flex: 1, fontSize: 14 },
  iconButton: { padding: 6 },
  apply: {
    alignItems: "center",
    backgroundColor: palette.accent,
    borderRadius: 999,
    marginTop: 24,
    paddingVertical: 14,
  },
  applyText: { color: palette.accentText, fontSize: 15, fontWeight: "700" },
});
