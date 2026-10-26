"use client";

import {
  DSP_SAMPLE_RATES,
  EQ_FILTER_TYPES,
  activeEqBands,
  crossfeedParameters,
  crossfeedStrengthLabels,
  defaultDspSettings,
  dspAvailability,
  dspMayClip,
  eqFilterTypeLabels,
  eqFrequencies,
  eqPresets,
  eqResponseDb,
  formatDecibels,
  formatFrequency,
  formatRealtimeFactor,
  formatSampleRate,
  recommendedHeadroomDb,
  SIGNAL_CLIPPING_COLOR,
  signalLightColor,
  signalLightLabel,
  type CrossfeedStrength,
  type DspAvailability,
  type DspSampleRateMode,
  type DspSettings,
  type EqBand,
  type EqFilterType,
  type OutputDspSupport,
  type SignalPath,
} from "@parson/music-sdk";
import {
  AlertTriangle,
  Gauge,
  Plus,
  RotateCcw,
  Server,
  Trash2,
} from "lucide-react";
import { useId, useMemo } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import { ConvolutionControls } from "./convolution-section";
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
  unavailable,
}: {
  title: string;
  description: string;
  enabled: boolean;
  onToggle: (enabled: boolean) => void;
  masterOn: boolean;
  children: React.ReactNode;
  badge?: React.ReactNode;
  /** Why this section cannot run on the current output, if it cannot. */
  unavailable?: string | null;
}) {
  const live = enabled && masterOn && !unavailable;
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
          {unavailable ? (
            <p
              className="mt-1 text-[11px] leading-4 text-amber-300/80"
              data-dsp-unavailable
            >
              {unavailable}
            </p>
          ) : null}
        </div>
        <Switch
          checked={enabled && !unavailable}
          disabled={Boolean(unavailable)}
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

function hasWebAudio() {
  return (
    typeof window !== "undefined" &&
    Boolean(
      window.AudioContext ||
      (window as typeof window & { webkitAudioContext?: unknown })
        .webkitAudioContext,
    )
  );
}

/**
 * Where a zone's DSP chain runs: in this browser (its own zone, with the
 * browser's output device picker), on the server before a zone's output
 * (devices, bridges, network players, served streams), or nowhere (Cast).
 */
export type DspPanelTarget =
  | { kind: "client"; output: PlayerOutputState }
  | {
      kind: "server";
      /** The zone's name. */
      name: string;
      /** The zone takes a DSP chain at all. */
      supported: boolean;
      /** What the engine can do for the zone's output. */
      support: OutputDspSupport | null;
    }
  | { kind: "cast" };

/** Where the chain runs for the chosen zone, and what it can do there. */
export function panelAvailability(
  target: DspPanelTarget,
  webAudio = hasWebAudio(),
): DspAvailability {
  if (target.kind === "cast") return dspAvailability({ kind: "cast" });
  if (target.kind === "server") {
    if (!target.supported)
      return {
        ...dspAvailability({ kind: "cast" }),
        reason: `${target.name} plays what its own app sends, so Parson cannot process it.`,
        engine: "Not available on this zone",
      };
    return dspAvailability({
      kind: "server",
      deviceName: target.name,
      support: target.support ?? undefined,
    });
  }
  return dspAvailability({ kind: "client", webAudio });
}

function sampleRateLabel(mode: DspSampleRateMode, deviceLabel: string) {
  if (mode === "device") return deviceLabel;
  if (mode === "source") return "Match source";
  return formatSampleRate(mode);
}

export function DspPanel({
  error = null,
  open,
  onOpenChange,
  path,
  settings,
  onChange,
  target,
}: {
  /** Saving the zone's chain failed. */
  error?: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  path: SignalPath | null;
  settings: DspSettings;
  onChange: (settings: DspSettings) => void;
  target: DspPanelTarget;
}) {
  const availability = panelAvailability(target);
  const blocked = availability.available ? null : availability.reason;
  const serverRun = target.kind === "server" && availability.available;
  const output = target.kind === "client" ? target.output : null;
  const status = path?.processing?.status ?? null;
  const outputSelectId = useId();
  const headroomId = useId();
  const update = (patch: Partial<DspSettings>) =>
    onChange({ ...settings, ...patch });
  const setBands = (bands: EqBand[], preset = "custom") =>
    update({ eq: { ...settings.eq, preset, bands } });
  const clipping = dspMayClip(settings);
  const recommended = recommendedHeadroomDb(settings);
  const outputName =
    target.kind === "server"
      ? target.name
      : (output?.label ?? "System default output");
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
              {availability.precision ? `, ${availability.engine}` : ""}. Each
              zone keeps its own settings.
            </DialogDescription>
          </div>
          <div className="flex items-center gap-2 pt-1 text-xs text-zinc-400">
            {settings.enabled ? "On" : "Bypassed"}
            <Switch
              checked={settings.enabled && !blocked}
              disabled={Boolean(blocked)}
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
            <QualityGlyph
              className="mt-0.5"
              clipping={path.clipping}
              quality={quality}
              size={14}
            />
            <p className="text-xs leading-5 text-zinc-400">
              <span
                className="font-semibold"
                style={{ color: signalLightColor(path) }}
              >
                {signalLightLabel(path)}
              </span>
              {". "}
              {path.description}
            </p>
          </div>
        ) : null}

        {serverRun && (status || path?.processing?.notes.length) ? (
          <div
            className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-white/6 px-5 py-2.5 text-[11px] text-zinc-400"
            data-dsp-processing
          >
            {status?.realtime_factor ? (
              <span className="inline-flex items-center gap-1.5">
                <Gauge className="h-3.5 w-3.5" />
                Processing speed{" "}
                <span
                  className="font-semibold tabular-nums text-zinc-100"
                  data-dsp-speed
                >
                  {formatRealtimeFactor(status.realtime_factor)}
                </span>
              </span>
            ) : null}
            {status?.peak_dbfs !== null && status?.peak_dbfs !== undefined ? (
              <span className="tabular-nums">
                Peak {formatDecibels(status.peak_dbfs)}FS
              </span>
            ) : null}
            {status?.clipping ? (
              <span
                className="inline-flex items-center gap-1 font-semibold"
                data-dsp-clipping
                style={{ color: SIGNAL_CLIPPING_COLOR }}
              >
                <AlertTriangle className="h-3.5 w-3.5" /> Clipping detected (
                {status.clipped_samples.toLocaleString()} samples)
                {recommended < 0 ? (
                  <button
                    className="ml-1 rounded-full border border-current px-2 py-0.5 font-medium hover:bg-white/6"
                    onClick={() =>
                      update({
                        headroom: {
                          enabled: true,
                          gainDb: Math.min(
                            recommended,
                            settings.headroom.enabled
                              ? settings.headroom.gainDb - 1
                              : recommended,
                          ),
                        },
                      })
                    }
                    type="button"
                  >
                    Add headroom
                  </button>
                ) : null}
              </span>
            ) : null}
            {path?.processing?.notes.map((note) => (
              <span className="w-full text-amber-300/80" key={note}>
                {note}
              </span>
            ))}
          </div>
        ) : null}

        <div className="space-y-3 px-5 py-4">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            {output ? (
              <>
                <label
                  className="text-xs font-medium text-zinc-300"
                  htmlFor={outputSelectId}
                >
                  Output device
                </label>
                <select
                  className={cn(fieldClass, "min-w-0 flex-1")}
                  disabled={!output.selectable}
                  id={outputSelectId}
                  onChange={(event) => output.select(event.target.value)}
                  value={output.selectedId}
                >
                  <option value="">
                    System default
                    {output.devices.find((device) => device.id === "default")
                      ?.label
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
              </>
            ) : null}
            {blocked ? (
              <p
                className="w-full rounded-lg bg-white/4 px-3 py-2 text-[11px] leading-4 text-amber-300/90"
                data-dsp-blocked
              >
                {blocked}
              </p>
            ) : null}
            {error ? (
              <p className="w-full text-[11px] text-red-300">{error}</p>
            ) : null}
            {serverRun ? (
              <p className="flex w-full items-start gap-1.5 text-[11px] leading-4 text-zinc-500">
                <Server className="mt-px h-3 w-3 shrink-0" />
                <span>
                  The server runs this chain itself, in 64-bit float, before{" "}
                  {target.name}; nothing after Parson mixes or resamples it
                  again. Set the zone's device mode and format limits in its
                  zone settings or in{" "}
                  <a
                    className="text-zinc-300 underline underline-offset-2"
                    href="/settings?tab=outputs"
                  >
                    Settings → Audio outputs
                  </a>
                  .
                </span>
              </p>
            ) : null}
            {output && !output.selectable ? (
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
            masterOn={settings.enabled && !blocked}
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
            description={
              serverRun
                ? "The rate the server sends to the device. Its converter is a linear-phase 64-bit polyphase sinc that runs after the EQ and crossfeed; the device receives that rate with no further conversion."
                : "The rate the processing graph runs at. Matching the source avoids a conversion; the device may still convert after Parson."
            }
            enabled={settings.sampleRate.enabled}
            masterOn={settings.enabled && !blocked}
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
                  <option
                    disabled={
                      typeof mode === "number" &&
                      availability.sampleRates !== null &&
                      !availability.sampleRates.includes(mode)
                    }
                    key={mode}
                    value={String(mode)}
                  >
                    {sampleRateLabel(mode, availability.deviceRateLabel)}
                    {typeof mode === "number" &&
                    availability.sampleRates !== null &&
                    !availability.sampleRates.includes(mode)
                      ? " (not supported by the device)"
                      : ""}
                  </option>
                ),
              )}
            </select>
          </Section>

          <Section
            description={`${activeEqBands({ ...settings, enabled: true, eq: { ...settings.eq, enabled: true } }).length} active of ${settings.eq.bands.length} bands. Biquad filters, ${availability.precision || "32-bit float"}.`}
            enabled={settings.eq.enabled}
            masterOn={settings.enabled && !blocked}
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
            masterOn={settings.enabled && !blocked}
            onToggle={(enabled) =>
              update({ crossfeed: { ...settings.crossfeed, enabled } })
            }
            title="Crossfeed"
            unavailable={
              availability.crossfeed.available
                ? null
                : availability.crossfeed.reason
            }
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

          <Section
            description="Convolves with an FIR filter you upload, such as a room correction from REW, Dirac or Acourate, after the EQ and crossfeed."
            enabled={settings.convolution?.enabled ?? false}
            masterOn={settings.enabled && !blocked}
            onToggle={(enabled) =>
              update({
                convolution: {
                  filterId: settings.convolution?.filterId ?? null,
                  enabled,
                },
              })
            }
            title="Convolution"
            unavailable={
              availability.convolution.available
                ? null
                : availability.convolution.reason
            }
          >
            <ConvolutionControls
              onChange={onChange}
              serverRun={serverRun}
              settings={settings}
            />
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
