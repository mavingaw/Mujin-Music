"use client";

import {
  CROSSFADE_CHOICES,
  LEVELING_ADJUSTMENT_RANGE,
  LEVELING_TARGET_RANGE,
  ZONE_ICONS,
  levelingModeLabel,
  zoneIconName,
  zoneKindLabel,
  zoneSettingsDraft,
  zoneSettingsPatch,
  zoneSettingsProblem,
  type Zone,
  type ZoneLevelingMode,
  type ZoneSettingsDraft,
} from "@parson/music-sdk";
import { Lock, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { ZONE_ICON_LABELS, ZoneIconGlyph } from "./zone-icons";
import { useZones } from "./zones-provider";

const LEVELING_MODES: ZoneLevelingMode[] = ["off", "auto", "track", "album"];
const LEVELING_HELP: Record<ZoneLevelingMode, string> = {
  off: "Tracks play at their own loudness.",
  auto: "Levels by album while an album plays in order, by track otherwise.",
  track: "Every track plays at the same loudness.",
  album: "Albums keep their own dynamics; whole albums are leveled.",
};

const sliderClass =
  "h-6 flex-1 **:data-[slot=slider-range]:bg-white **:data-[slot=slider-thumb]:h-3.5 **:data-[slot=slider-thumb]:w-3.5 **:data-[slot=slider-thumb]:border-0 **:data-[slot=slider-thumb]:bg-white **:data-[slot=slider-track]:h-1 **:data-[slot=slider-track]:bg-white/15";

export function errorText(error: unknown, fallback: string): string {
  const code = (
    error as { response?: { data?: { error?: string; message?: string } } }
  )?.response?.data?.error;
  const known: Record<string, string> = {
    set_private_on_the_device_itself:
      "A zone can only be made private from the device it runs on.",
    only_parson_apps_can_be_private:
      "Only Parson apps can be private; speakers are shared by everyone.",
    crossfade_not_supported_here: "This zone cannot crossfade.",
    leveling_not_supported_here: "This zone cannot level volume.",
    comfort_outside_safety_limits:
      "The comfort limit must sit inside the safety limits.",
    invalid_safety_limits: "The safety minimum must be below the maximum.",
    zone_name_required: "Give the zone a name.",
  };
  return (code && known[code]) || (code ? code.replaceAll("_", " ") : fallback);
}

/** Roon-style Zone settings: name, icon, crossfade, leveling, volume limits and privacy. */
export function ZoneSettingsDialog({
  open,
  onOpenChange,
  zone,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  zone: Zone | null;
}) {
  return (
    <Dialog open={open && Boolean(zone)} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg gap-0 p-0 sm:p-0">
        {zone && (
          <ZoneSettingsForm
            key={zone.id}
            onDone={() => onOpenChange(false)}
            zone={zone}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function ZoneSettingsForm({
  onDone,
  zone,
}: {
  onDone: () => void;
  zone: Zone;
}) {
  const zones = useZones();
  const initial = useMemo(() => zoneSettingsDraft(zone), [zone]);
  const [draft, setDraft] = useState<ZoneSettingsDraft>(initial);
  const [saving, setSaving] = useState(false);
  // A rename or limit change made elsewhere shows up while untouched.
  const [touched, setTouched] = useState(false);
  useEffect(() => {
    if (!touched) setDraft(initial);
  }, [initial, touched]);
  const update = (change: Partial<ZoneSettingsDraft>) => {
    setTouched(true);
    setDraft((current) => ({ ...current, ...change }));
  };
  const settings = zone.settings;
  const supports = settings?.supports ?? {
    crossfade: false,
    leveling: false,
    volume_limits: Boolean(zone.volume),
    private: false,
  };
  const patch = zoneSettingsPatch(initial, draft);
  const changed = Object.keys(patch).length > 0;
  const problem = zoneSettingsProblem(draft);
  const isThisDevice = zone.id === zones.localZoneId;
  const defaultName = settings?.default_name ?? zone.name;
  const safetyMin = draft.safety_min_volume ?? 0;
  const safetyMax = draft.safety_max_volume ?? 100;

  const save = async () => {
    if (!changed || problem) return;
    setSaving(true);
    try {
      await zones.updateSettings(zone.id, patch);
      toast.success("Zone settings saved");
      onDone();
    } catch (error) {
      toast.error(errorText(error, "Could not save the zone settings."));
    } finally {
      setSaving(false);
    }
  };

  return (
    <form
      className="flex max-h-[calc(100dvh-2rem)] flex-col"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <DialogHeader className="border-b border-white/10 px-5 pb-3 pt-5 text-left">
        <DialogTitle>Zone settings</DialogTitle>
        <DialogDescription>
          {zone.device || zoneKindLabel(zone.kind)}
          {isThisDevice ? " · This device" : ""}
        </DialogDescription>
      </DialogHeader>
      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-5 py-4">
        <Section title="Zone">
          <div className="flex items-start gap-4">
            <label className="min-w-0 flex-1 text-xs text-zinc-400">
              Zone name
              <Input
                aria-label="Zone name"
                className="mt-1 h-9"
                maxLength={80}
                onChange={(event) => update({ name: event.target.value })}
                placeholder={defaultName}
                value={draft.name}
              />
              <span className="mt-1 flex items-center gap-2 text-[11px] text-zinc-600">
                <span className="truncate">Device name: {defaultName}</span>
                {draft.name.trim() !== defaultName && (
                  <button
                    className="inline-flex items-center gap-1 text-zinc-400 underline-offset-2 hover:text-white hover:underline"
                    onClick={() => update({ name: defaultName })}
                    type="button"
                  >
                    <RotateCcw className="h-3 w-3" /> Use device name
                  </button>
                )}
              </span>
            </label>
            <span
              aria-hidden
              className="mt-5 flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-white/[0.06] text-zinc-200"
            >
              <ZoneIconGlyph
                className="h-6 w-6"
                name={zoneIconName({
                  kind: zone.kind,
                  icon: draft.icon ?? undefined,
                })}
              />
            </span>
          </div>
          <fieldset className="mt-3">
            <legend className="text-xs text-zinc-400">Zone icon</legend>
            <div className="mt-1.5 grid grid-cols-6 gap-1.5 sm:grid-cols-9">
              <IconChoice
                label="Device icon"
                onSelect={() => update({ icon: null })}
                selected={draft.icon === null}
              >
                <ZoneIconGlyph
                  name={zoneIconName({ kind: zone.kind })}
                  className="h-4 w-4 opacity-60"
                />
              </IconChoice>
              {ZONE_ICONS.map((icon) => (
                <IconChoice
                  key={icon}
                  label={ZONE_ICON_LABELS[icon]}
                  onSelect={() => update({ icon })}
                  selected={draft.icon === icon}
                >
                  <ZoneIconGlyph name={icon} />
                </IconChoice>
              ))}
            </div>
          </fieldset>
        </Section>

        <Section title="Zone options">
          <Row
            label="Crossfade time"
            note={
              supports.crossfade ? undefined : "Not available on this zone."
            }
          >
            <Select
              disabled={!supports.crossfade}
              onValueChange={(value) =>
                update({ crossfade_seconds: Number(value) })
              }
              value={String(draft.crossfade_seconds)}
            >
              <SelectTrigger aria-label="Crossfade time" className="h-9 w-36">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CROSSFADE_CHOICES.map((seconds) => (
                  <SelectItem key={seconds} value={String(seconds)}>
                    {seconds === 0 ? "Off" : `${seconds} seconds`}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          <Row
            label="Volume leveling"
            note={
              supports.leveling
                ? LEVELING_HELP[draft.leveling]
                : "Not available on this zone."
            }
          >
            <Select
              disabled={!supports.leveling}
              onValueChange={(value) =>
                update({ leveling: value as ZoneLevelingMode })
              }
              value={draft.leveling}
            >
              <SelectTrigger aria-label="Volume leveling" className="h-9 w-36">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LEVELING_MODES.map((mode) => (
                  <SelectItem key={mode} value={mode}>
                    {levelingModeLabel(mode)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Row>
          {supports.leveling && draft.leveling !== "off" && (
            <div className="space-y-3 rounded-lg border border-white/[0.06] p-3">
              <SliderRow
                label="Target"
                max={LEVELING_TARGET_RANGE.max}
                min={LEVELING_TARGET_RANGE.min}
                onChange={(value) => update({ leveling_target_lufs: value })}
                unit="LUFS"
                value={draft.leveling_target_lufs}
              />
              <SliderRow
                label="Adjustment"
                max={LEVELING_ADJUSTMENT_RANGE.max}
                min={LEVELING_ADJUSTMENT_RANGE.min}
                onChange={(value) => update({ leveling_adjustment_db: value })}
                signed
                unit="dB"
                value={draft.leveling_adjustment_db}
              />
              <p className="text-[11px] leading-4 text-zinc-500">
                Loudness is measured once per track (EBU R128). Peaks stay under
                -1 dBTP, so quiet tracks are only raised as far as they can go
                without clipping.
              </p>
            </div>
          )}
        </Section>

        {supports.volume_limits && (
          <Section title="Volume limits">
            <div className="space-y-4">
              <div>
                <label className="flex items-center gap-2 text-sm text-zinc-200">
                  <Checkbox
                    aria-label="Comfort limit"
                    checked={draft.comfort_volume !== null}
                    onCheckedChange={(value) =>
                      update({
                        comfort_volume:
                          value === true
                            ? Math.min(Math.max(50, safetyMin), safetyMax)
                            : null,
                      })
                    }
                  />
                  Comfort limit
                </label>
                <p className="ml-6 mt-0.5 text-[11px] leading-4 text-zinc-500">
                  Volume sliders stop here; the + button can still go past it.
                </p>
                {draft.comfort_volume !== null && (
                  <div className="ml-6 mt-2">
                    <SliderRow
                      label="Comfort"
                      max={safetyMax}
                      min={safetyMin}
                      onChange={(value) => update({ comfort_volume: value })}
                      value={draft.comfort_volume}
                    />
                  </div>
                )}
              </div>
              <div>
                <p className="text-sm text-zinc-200">Safety limits</p>
                <p className="mt-0.5 text-[11px] leading-4 text-zinc-500">
                  Hard limits: nothing sets the volume outside this range.
                </p>
                <div className="mt-2 space-y-2">
                  <SliderRow
                    label="Minimum"
                    max={99}
                    min={0}
                    onChange={(low) => {
                      const high = Math.max(safetyMax, low + 1);
                      update({
                        safety_min_volume: low > 0 ? low : null,
                        safety_max_volume: high < 100 ? high : null,
                        comfort_volume: clampComfort(
                          draft.comfort_volume,
                          low,
                          high,
                        ),
                      });
                    }}
                    value={safetyMin}
                  />
                  <SliderRow
                    label="Maximum"
                    max={100}
                    min={1}
                    onChange={(high) => {
                      const low = Math.min(safetyMin, high - 1);
                      update({
                        safety_min_volume: low > 0 ? low : null,
                        safety_max_volume: high < 100 ? high : null,
                        comfort_volume: clampComfort(
                          draft.comfort_volume,
                          low,
                          high,
                        ),
                      });
                    }}
                    value={safetyMax}
                  />
                </div>
              </div>
              <div className="flex items-center justify-between text-sm">
                <span className="text-zinc-200">Device range</span>
                <span className="text-xs tabular-nums text-zinc-500">
                  {zone.volume?.device_min ?? 0}–
                  {zone.volume?.device_max ?? 100}
                  {" · reported by the device"}
                </span>
              </div>
            </div>
          </Section>
        )}

        {supports.private && (
          <Section title="Privacy">
            <label
              className={`flex items-start gap-2 text-sm ${isThisDevice ? "text-zinc-200" : "text-zinc-500"}`}
            >
              <Checkbox
                aria-label="Private zone"
                checked={draft.private}
                className="mt-0.5"
                disabled={!isThisDevice}
                onCheckedChange={(value) => update({ private: value === true })}
              />
              <span>
                <span className="flex items-center gap-1.5">
                  <Lock className="h-3.5 w-3.5" /> Private zone
                </span>
                <span className="mt-0.5 block text-[11px] leading-4 text-zinc-500">
                  {isThisDevice
                    ? "Only this device sees and controls this zone. Other remotes cannot see it, and it stays out of groups."
                    : "Set this on the device the zone runs on."}
                </span>
              </span>
            </label>
          </Section>
        )}

        {settings?.notes && settings.notes.length > 0 && (
          <ul className="space-y-1">
            {settings.notes.map((note) => (
              <li className="text-[11px] leading-4 text-zinc-500" key={note}>
                {note}
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex items-center justify-between gap-3 border-t border-white/10 px-5 py-3">
        <p className="min-w-0 text-[11px] text-amber-300" role="status">
          {problem}
        </p>
        <div className="flex shrink-0 gap-2">
          <button
            className="h-9 rounded-full px-4 text-sm text-zinc-400 hover:text-white"
            onClick={onDone}
            type="button"
          >
            Cancel
          </button>
          <button
            className="h-9 rounded-full bg-white px-4 text-sm font-semibold text-black disabled:opacity-40"
            disabled={!changed || Boolean(problem) || saving}
            type="submit"
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </form>
  );
}

function clampComfort(
  comfort: number | null,
  low: number,
  high: number,
): number | null {
  return comfort === null ? null : Math.min(Math.max(comfort, low), high);
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">
        {title}
      </h3>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

function Row({
  label,
  note,
  children,
}: {
  label: string;
  note?: string;
  children: ReactNode;
}) {
  return (
    <div>
      <div className="flex items-center justify-between gap-3">
        <span className="text-sm text-zinc-200">{label}</span>
        {children}
      </div>
      {note && (
        <p className="mt-0.5 text-[11px] leading-4 text-zinc-500">{note}</p>
      )}
    </div>
  );
}

function SliderRow({
  label,
  max,
  min,
  onChange,
  signed = false,
  unit,
  value,
}: {
  label: string;
  max: number;
  min: number;
  onChange: (value: number) => void;
  signed?: boolean;
  unit?: string;
  value: number;
}) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-20 text-xs text-zinc-400">{label}</span>
      <Slider
        aria-label={label}
        className={sliderClass}
        max={max}
        min={min}
        onValueChange={([next]) => onChange(next ?? value)}
        step={1}
        value={[value]}
      />
      <span className="w-16 text-right text-xs tabular-nums text-zinc-300">
        {signed && value > 0 ? "+" : ""}
        {value}
        {unit ? ` ${unit}` : ""}
      </span>
    </div>
  );
}

function IconChoice({
  children,
  label,
  onSelect,
  selected,
}: {
  children: ReactNode;
  label: string;
  onSelect: () => void;
  selected: boolean;
}) {
  return (
    <button
      aria-label={label}
      aria-pressed={selected}
      className={`flex h-9 items-center justify-center rounded-lg border ${selected ? "border-white bg-white/15 text-white" : "border-white/10 text-zinc-400 hover:border-white/25 hover:text-white"}`}
      onClick={onSelect}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}
