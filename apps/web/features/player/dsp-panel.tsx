"use client";

import {
  DSP_SAMPLE_RATES,
  EQ_FILTER_TYPES,
  activeEqBands,
  crossfeedParameters,
  crossfeedStrengthLabels,
  defaultDspSettings,
  dspMayClip,
  eqFilterTypeLabels,
  eqFrequencies,
  eqPresets,
  eqResponseDb,
  formatDecibels,
  formatFrequency,
  formatSampleRate,
  recommendedHeadroomDb,
  signalQualityColors,
  signalQualityLabels,
  type CrossfeedStrength,
  type DspSampleRateMode,
  type DspSettings,
  type EqBand,
  type EqFilterType,
  type SignalPath,
} from "@parson/music-sdk";
import { AlertTriangle, Plus, RotateCcw, Trash2 } from "lucide-react";
import { useId, useMemo } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import type { PlayerOutputState } from "./player-api";
import { QualityGlyph } from "./signal-path-popover";

export const DSP_SHORTCUT_LABEL =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform)
    ? "⌘/"
    : "Ctrl+/";

/** Ctrl+/ (Cmd+/ on a Mac) opens the DSP engine, as in Roon. */
export function isDspShortcut(
  event: Pick<
    KeyboardEvent,
    "code" | "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey"
  >,
) {
  return (
    (event.ctrlKey || event.metaKey) &&
    !event.altKey &&
    !event.shiftKey &&
    (event.code === "Slash" || event.key === "/")
  );
}

function Switch({
  checked,
  label,
  onChange,
  disabled,
}: {
  checked: boolean;
  label: string;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={cn(
        "relative inline-flex h-5.5 w-9.5 shrink-0 items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/50 disabled:opacity-40",
        checked ? "bg-[#38b6ff]" : "bg-zinc-700",
      )}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      role="switch"
      type="button"
    >
      <span
        className={cn(
          "inline-block h-4.5 w-4.5 rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-4.5" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

function Section({
  title,
  description,
  enabled,
  onToggle,
  masterOn,
  children,
  badge,
}: {
  title: string;
  description: string;
  enabled: boolean;
  onToggle: (enabled: boolean) => void;
  masterOn: boolean;
  children: React.ReactNode;
  badge?: React.ReactNode;
}) {
  const live = enabled && masterOn;
  return (
    <section
      className={cn(
        "rounded-xl border px-4 py-3 transition-colors",
        live ? "border-[#38b6ff]/30 bg-[#38b6ff]/4" : "border-white/8",
      )}
      data-dsp-section={title}
    >
      <header className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            {title}
            {badge}
          </h3>
          <p className="mt-0.5 text-xs leading-5 text-zinc-500">
            {description}
          </p>
        </div>
        <Switch
          checked={enabled}
          label={`${title} ${enabled ? "on" : "off"}`}
          onChange={onToggle}
        />
      </header>
      <div
        className={cn("mt-3", !live && "pointer-events-none opacity-45")}
        aria-disabled={!live}
      >
        {children}
      </div>
    </section>
  );
}

const fieldClass =
  "h-8 rounded-md border border-white/10 bg-[#141416] px-2 text-xs text-zinc-100 focus:outline-none focus:ring-1 focus:ring-white/40";

function EqCurve({ settings }: { settings: DspSettings }) {
  const points = useMemo(() => {
    const bands = activeEqBands({
      ...settings,
      enabled: true,
      eq: { ...settings.eq, enabled: true },
    });
    const frequencies = eqFrequencies(120);
    return frequencies.map((frequency, index) => {
      const db = bands.length ? eqResponseDb(bands, frequency) : 0;
      const x = (index / (frequencies.length - 1)) * 100;
      const y = 50 - (Math.max(-12, Math.min(12, db)) / 12) * 50;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    });
  }, [settings]);
  return (
    <svg
      aria-label="EQ response curve"
      className="h-24 w-full rounded-lg bg-black/40"
      preserveAspectRatio="none"
      role="img"
      viewBox="0 0 100 100"
    >
      {[-6, 0, 6].map((db) => {
        const y = 50 - (db / 12) * 50;
        return (
          <line
            key={db}
            stroke={db === 0 ? "#52525b" : "#27272a"}
            strokeWidth={0.4}
            vectorEffect="non-scaling-stroke"
            x1={0}
            x2={100}
            y1={y}
            y2={y}
          />
        );
      })}
      {[100, 1_000, 10_000].map((frequency) => {
        const x =
          ((Math.log10(frequency) - Math.log10(20)) /
            (Math.log10(20_000) - Math.log10(20))) *
          100;
        return (
          <line
            key={frequency}
            stroke="#27272a"
            strokeWidth={0.4}
            vectorEffect="non-scaling-stroke"
            x1={x}
            x2={x}
            y1={0}
            y2={100}
          />
        );
      })}
      <polyline
        fill="none"
        points={points.join(" ")}
        stroke="#38b6ff"
        strokeWidth={2}
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

function BandRow({
  band,
  index,
  onChange,
  onRemove,
}: {
  band: EqBand;
  index: number;
  onChange: (band: EqBand) => void;
  onRemove: () => void;
}) {
  const hasGain = ["peaking", "lowshelf", "highshelf"].includes(band.type);
  const label = `Band ${index + 1}`;
  return (
    <li
      className="grid grid-cols-[auto_1fr_auto] items-center gap-x-2 gap-y-1.5 rounded-lg bg-white/3 px-2 py-2 sm:grid-cols-[auto_7rem_5.5rem_1fr_4rem_auto]"
      data-band={band.id}
    >
      <input
        aria-label={`${label} enabled`}
        checked={band.enabled}
        className="h-4 w-4 accent-[#38b6ff]"
        onChange={(event) =>
          onChange({ ...band, enabled: event.target.checked })
        }
        type="checkbox"
      />
      <select
        aria-label={`${label} filter type`}
        className={fieldClass}
        onChange={(event) =>
          onChange({ ...band, type: event.target.value as EqFilterType })
        }
        value={band.type}
      >
        {EQ_FILTER_TYPES.map((type) => (
          <option key={type} value={type}>
            {eqFilterTypeLabels[type]}
          </option>
        ))}
      </select>
      <button
        aria-label={`Remove ${label.toLowerCase()}`}
        className="flex h-8 w-8 items-center justify-center rounded-md text-zinc-500 hover:bg-white/8 hover:text-white sm:order-last"
        onClick={onRemove}
        type="button"
      >
        <Trash2 className="h-3.5 w-3.5" />
      </button>
      <label className="col-span-3 flex items-center gap-1 text-[11px] text-zinc-500 sm:col-span-1">
        <input
          aria-label={`${label} frequency in hertz`}
          className={cn(fieldClass, "w-full")}
          max={20_000}
          min={20}
          onChange={(event) => {
            const value = Number(event.target.value);
            if (Number.isFinite(value) && value > 0)
              onChange({
                ...band,
                frequency: Math.min(20_000, Math.max(20, value)),
              });
          }}
          step={1}
          type="number"
          value={Math.round(band.frequency)}
        />
        Hz
      </label>
      <label className="col-span-3 flex items-center gap-2 sm:col-span-1">
        <input
          aria-label={`${label} gain in decibels`}
          className="h-1.5 w-full accent-[#38b6ff] disabled:opacity-30"
          disabled={!hasGain}
          max={12}
          min={-12}
          onChange={(event) =>
            onChange({ ...band, gainDb: Number(event.target.value) })
          }
          step={0.5}
          type="range"
          value={band.gainDb}
        />
        <span className="w-14 shrink-0 text-right text-xs tabular-nums text-zinc-300">
          {hasGain
            ? `${band.gainDb > 0 ? "+" : ""}${band.gainDb.toFixed(1)} dB`
            : "—"}
        </span>
      </label>
      <label className="col-span-3 flex items-center gap-1 text-[11px] text-zinc-500 sm:col-span-1">
        Q
        <input
          aria-label={`${label} Q`}
          className={cn(fieldClass, "w-full")}
          max={18}
          min={0.1}
          onChange={(event) => {
            const value = Number(event.target.value);
            if (Number.isFinite(value) && value > 0)
              onChange({ ...band, q: Math.min(18, Math.max(0.1, value)) });
          }}
          step={0.1}
          type="number"
          value={band.q}
        />
      </label>
    </li>
  );
}

function sampleRateLabel(mode: DspSampleRateMode) {
  if (mode === "device") return "Output device rate";
  if (mode === "source") return "Match source";
  return formatSampleRate(mode);
}

export function DspPanel({
  open,
  onOpenChange,
  output,
  path,
  settings,
  onChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  output: PlayerOutputState;
  path: SignalPath | null;
  settings: DspSettings;
  onChange: (settings: DspSettings) => void;
}) {
  const outputSelectId = useId();
  const headroomId = useId();
  const update = (patch: Partial<DspSettings>) =>
    onChange({ ...settings, ...patch });
  const setBands = (bands: EqBand[], preset = "custom") =>
    update({ eq: { ...settings.eq, preset, bands } });
  const clipping = dspMayClip(settings);
  const recommended = recommendedHeadroomDb(settings);
  const outputName = output.label ?? "System default output";
  const quality = path?.quality ?? null;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-2xl gap-0 p-0"
        data-dsp-panel
      >
        <header className="flex items-start gap-3 border-b border-white/6 px-5 pb-4 pt-5 pr-14">
          <div className="min-w-0 flex-1">
            <DialogTitle className="text-lg font-semibold">
              DSP engine
            </DialogTitle>
            <DialogDescription className="mt-0.5 text-xs text-zinc-500">
              Processing for <span className="text-zinc-300">{outputName}</span>
              . Each output device keeps its own settings.
            </DialogDescription>
          </div>
          <div className="flex items-center gap-2 pt-1 text-xs text-zinc-400">
            {settings.enabled ? "On" : "Bypassed"}
            <Switch
              checked={settings.enabled}
              label={`DSP engine ${settings.enabled ? "on" : "bypassed"}`}
              onChange={(enabled) => update({ enabled })}
            />
          </div>
        </header>

        {path ? (
          <div
            className="flex items-start gap-2.5 border-b border-white/6 px-5 py-3"
            data-dsp-light
          >
            <QualityGlyph className="mt-0.5" quality={quality} size={14} />
            <p className="text-xs leading-5 text-zinc-400">
              <span
                className="font-semibold"
                style={{
                  color: quality ? signalQualityColors[quality] : undefined,
                }}
              >
                {quality ? signalQualityLabels[quality] : "Unknown"}
              </span>
              {". "}
              {path.description}
            </p>
          </div>
        ) : null}

        <div className="space-y-3 px-5 py-4">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            <label
              className="text-xs font-medium text-zinc-300"
              htmlFor={outputSelectId}
            >
              Output device
            </label>
            <select
              className={cn(fieldClass, "min-w-0 flex-1")}
              disabled={!output.selectable && !output.serverDevices.length}
              id={outputSelectId}
              onChange={(event) => output.select(event.target.value)}
              value={output.selectedId}
            >
              <option value="">
                System default
                {output.devices.find((device) => device.id === "default")?.label
                  ? ` (${output.devices
                      .find((device) => device.id === "default")!
                      .label.replace(/^Default\s*[-–]\s*/i, "")})`
                  : ""}
              </option>
              {output.selectable
                ? output.devices
                    .filter((device) => device.id !== "default")
                    .map((device, index) => (
                      <option key={device.id} value={device.id}>
                        {device.label || `Output ${index + 1}`}
                      </option>
                    ))
                : null}
              {output.serverDevices.length ? (
                <optgroup label="Server outputs (exclusive, bit-perfect)">
                  {output.serverDevices.map((device) => (
                    <option
                      disabled={!device.formats}
                      key={device.id}
                      value={`server:${device.id}`}
                    >
                      {device.name} · {device.mode_label}
                    </option>
                  ))}
                </optgroup>
              ) : null}
            </select>
            {output.labelsHidden ? (
              <button
                className="h-8 rounded-md border border-white/10 px-2.5 text-xs text-zinc-300 hover:bg-white/6"
                onClick={() => void output.requestLabels()}
                type="button"
              >
                Show device names
              </button>
            ) : null}
            {output.serverOutputId ? (
              <p className="w-full text-[11px] leading-4 text-zinc-500">
                The server plays this output itself, with fixed volume, so the
                browser DSP below does not apply. Set Exclusive Mode and format
                limits in{" "}
                <a
                  className="text-zinc-300 underline underline-offset-2"
                  href="/settings?tab=outputs"
                >
                  Settings → Audio outputs
                </a>
                .
              </p>
            ) : !output.selectable ? (
              <p className="w-full text-[11px] text-zinc-500">
                This browser always plays through the system default output.
              </p>
            ) : null}
          </div>

          <Section
            badge={
              clipping ? (
                <span className="inline-flex items-center gap-1 rounded-full bg-amber-400/12 px-2 text-[10px] font-semibold text-amber-300">
                  <AlertTriangle className="h-3 w-3" /> May clip
                </span>
              ) : null
            }
            description="Lowers the level before the EQ so boosts cannot clip."
            enabled={settings.headroom.enabled}
            masterOn={settings.enabled}
            onToggle={(enabled) =>
              update({ headroom: { ...settings.headroom, enabled } })
            }
            title="Headroom management"
          >
            <div className="flex items-center gap-3">
              <input
                aria-label="Headroom adjustment in decibels"
                className="h-1.5 flex-1 accent-[#38b6ff]"
                id={headroomId}
                max={0}
                min={-12}
                onChange={(event) =>
                  update({
                    headroom: {
                      ...settings.headroom,
                      gainDb: Number(event.target.value),
                    },
                  })
                }
                step={0.5}
                type="range"
                value={settings.headroom.gainDb}
              />
              <span className="w-16 text-right text-xs tabular-nums text-zinc-200">
                {formatDecibels(settings.headroom.gainDb)}
              </span>
            </div>
            {clipping ? (
              <p className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-amber-300/90">
                The EQ boosts up to {Math.abs(recommended).toFixed(1)} dB.
                <button
                  className="rounded-full border border-amber-300/30 px-2 py-0.5 hover:bg-amber-300/10"
                  onClick={() =>
                    update({
                      headroom: { enabled: true, gainDb: recommended },
                    })
                  }
                  type="button"
                >
                  Use {formatDecibels(recommended)}
                </button>
              </p>
            ) : null}
          </Section>

          <Section
            description="The rate the processing graph runs at. Matching the source avoids a conversion; the device may still convert after Parson."
            enabled={settings.sampleRate.enabled}
            masterOn={settings.enabled}
            onToggle={(enabled) =>
              update({ sampleRate: { ...settings.sampleRate, enabled } })
            }
            title="Sample rate conversion"
          >
            <select
              aria-label="Processing sample rate"
              className={cn(fieldClass, "w-full")}
              onChange={(event) => {
                const value = event.target.value;
                const mode: DspSampleRateMode =
                  value === "device" || value === "source"
                    ? value
                    : Number(value);
                update({ sampleRate: { ...settings.sampleRate, mode } });
              }}
              value={String(settings.sampleRate.mode)}
            >
              {(["device", "source", ...DSP_SAMPLE_RATES] as const).map(
                (mode) => (
                  <option key={mode} value={String(mode)}>
                    {sampleRateLabel(mode)}
                  </option>
                ),
              )}
            </select>
          </Section>

          <Section
            description={`${activeEqBands({ ...settings, enabled: true, eq: { ...settings.eq, enabled: true } }).length} active of ${settings.eq.bands.length} bands. Biquad filters, 32-bit float.`}
            enabled={settings.eq.enabled}
            masterOn={settings.enabled}
            onToggle={(enabled) => update({ eq: { ...settings.eq, enabled } })}
            title="Parametric EQ"
          >
            <div className="mb-2 flex items-center gap-2">
              <select
                aria-label="EQ preset"
                className={cn(fieldClass, "flex-1")}
                onChange={(event) => {
                  const preset = eqPresets.find(
                    (item) => item.id === event.target.value,
                  );
                  if (preset)
                    setBands(
                      preset.bands.map((band) => ({ ...band })),
                      preset.id,
                    );
                }}
                value={
                  eqPresets.some((preset) => preset.id === settings.eq.preset)
                    ? settings.eq.preset
                    : "custom"
                }
              >
                {eqPresets.map((preset) => (
                  <option key={preset.id} value={preset.id}>
                    {preset.label}
                  </option>
                ))}
                <option disabled value="custom">
                  Custom
                </option>
              </select>
              <button
                aria-label="Reset EQ"
                className="flex h-8 w-8 items-center justify-center rounded-md text-zinc-400 hover:bg-white/8 hover:text-white"
                onClick={() => setBands(defaultDspSettings().eq.bands, "flat")}
                type="button"
              >
                <RotateCcw className="h-3.5 w-3.5" />
              </button>
            </div>
            <EqCurve settings={settings} />
            <div className="mt-1 flex justify-between text-[10px] text-zinc-600">
              <span>20 Hz</span>
              <span>{formatFrequency(1_000)}</span>
              <span>20 kHz</span>
            </div>
            <ol aria-label="EQ bands" className="mt-2 space-y-1.5">
              {settings.eq.bands.map((band, index) => (
                <BandRow
                  band={band}
                  index={index}
                  key={band.id}
                  onChange={(next) =>
                    setBands(
                      settings.eq.bands.map((item) =>
                        item.id === band.id ? next : item,
                      ),
                    )
                  }
                  onRemove={() =>
                    setBands(
                      settings.eq.bands.filter((item) => item.id !== band.id),
                    )
                  }
                />
              ))}
            </ol>
            {settings.eq.bands.length < 10 ? (
              <button
                className="mt-2 inline-flex h-8 items-center gap-1.5 rounded-md px-2 text-xs text-zinc-300 hover:bg-white/6"
                onClick={() =>
                  setBands([
                    ...settings.eq.bands,
                    {
                      id: `band-${Date.now().toString(36)}`,
                      enabled: true,
                      type: "peaking",
                      frequency: 2_000,
                      gainDb: 0,
                      q: 1,
                    },
                  ])
                }
                type="button"
              >
                <Plus className="h-3.5 w-3.5" /> Add band
              </button>
            ) : null}
          </Section>

          <Section
            description="Feeds a delayed, low-passed share of each channel to the other ear for more natural headphone listening."
            enabled={settings.crossfeed.enabled}
            masterOn={settings.enabled}
            onToggle={(enabled) =>
              update({ crossfeed: { ...settings.crossfeed, enabled } })
            }
            title="Crossfeed"
          >
            <div
              aria-label="Crossfeed strength"
              className="flex rounded-full bg-white/6 p-0.5"
              role="radiogroup"
            >
              {(Object.keys(crossfeedParameters) as CrossfeedStrength[]).map(
                (strength) => {
                  const params = crossfeedParameters[strength];
                  return (
                    <button
                      aria-checked={settings.crossfeed.strength === strength}
                      className={cn(
                        "h-7 flex-1 rounded-full text-[11px] font-medium transition-colors",
                        settings.crossfeed.strength === strength
                          ? "bg-white text-black"
                          : "text-zinc-400 hover:text-white",
                      )}
                      key={strength}
                      onClick={() =>
                        update({
                          crossfeed: { ...settings.crossfeed, strength },
                        })
                      }
                      role="radio"
                      title={`${params.cutoffHz} Hz, ${params.feedDb} dB, ${params.delayMs} ms`}
                      type="button"
                    >
                      {crossfeedStrengthLabels[strength]}
                    </button>
                  );
                },
              )}
            </div>
          </Section>
        </div>
        <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-white/6 px-5 py-3 text-[11px] text-zinc-500">
          <span>
            Press{" "}
            <kbd className="rounded border border-white/12 px-1 font-sans text-zinc-300">
              {DSP_SHORTCUT_LABEL}
            </kbd>{" "}
            to open the DSP engine from anywhere.
          </span>
          <span>Every stage appears in the signal path.</span>
        </footer>
      </DialogContent>
    </Dialog>
  );
}
