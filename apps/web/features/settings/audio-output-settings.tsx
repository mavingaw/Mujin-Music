"use client";

import {
  dsdRateLabel,
  DSD_STRATEGIES,
  nativeFormatLabel,
  refreshNativeOutputs,
  updateNativeOutputSettings,
  type DsdStrategy,
  type FormatSupportEntry,
  type NativeOutputDevice,
  type NativeOutputSettings,
  type NativeOutputs,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, HardDrive, RefreshCw, Speaker, X } from "lucide-react";
import { useId } from "react";
import { cn } from "@/lib/cn";
import {
  nativeOutputsKey,
  SERVER_OUTPUT_PREFIX,
  useNativeOutputs,
} from "@/features/player/use-native-outputs";
import { useOutputDeviceChoice } from "@/features/player/dsp-store";
import {
  descriptorLines,
  deviceCapabilitySummary,
  driverLabels,
  dsdStrategyNote,
  dsdSupportSummary,
  formatTone,
  formatToneLabel,
  hardwareSummary,
  maxDepthOptions,
  maxDsdRateOptions,
  maxRateOptions,
} from "./audio-outputs-model";

const selectClass =
  "h-9 rounded-md border border-white/10 bg-zinc-900 px-2.5 text-sm text-zinc-100 disabled:opacity-50";

function Toggle({
  checked,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={cn(
        "relative inline-flex h-5.5 w-9.5 shrink-0 items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/50 disabled:opacity-40",
        checked ? "bg-[#b26bff]" : "bg-zinc-700",
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

function FormatChip({
  entry,
  label,
}: {
  entry: FormatSupportEntry;
  label: string;
}) {
  const tone = formatTone(entry);
  return (
    <li
      className={cn(
        "flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs tabular-nums",
        tone === "supported" &&
          "border-[#b26bff]/35 bg-[#b26bff]/10 text-zinc-100",
        tone === "capped" && "border-white/10 bg-white/3 text-zinc-500",
        tone === "unsupported" &&
          "border-red-500/35 bg-red-500/10 text-red-400",
      )}
      data-format-tone={tone}
      title={formatToneLabel(tone)}
    >
      {tone === "unsupported" ? (
        <X aria-hidden className="h-3 w-3" />
      ) : (
        <Check aria-hidden className="h-3 w-3" />
      )}
      {label}
      <span className="sr-only">: {formatToneLabel(tone)}</span>
    </li>
  );
}

function DeviceCard({
  device,
  isAdmin,
  selected,
  onUse,
}: {
  device: NativeOutputDevice;
  isAdmin: boolean;
  selected: boolean;
  onUse: () => void;
}) {
  const queryClient = useQueryClient();
  const rateId = useId();
  const depthId = useId();
  const strategyId = useId();
  const dsdRateId = useId();
  const save = useMutation({
    mutationFn: (settings: NativeOutputSettings) =>
      updateNativeOutputSettings(device.id, settings),
    onSuccess: (updated) => {
      queryClient.setQueryData<NativeOutputs>(nativeOutputsKey, (current) =>
        current
          ? {
              ...current,
              devices: current.devices.map((item) =>
                item.id === updated.id ? updated : item,
              ),
            }
          : current,
      );
      void queryClient.invalidateQueries({ queryKey: ["signal-path"] });
    },
  });
  const settings = device.settings;
  const change = (patch: Partial<NativeOutputSettings>) =>
    save.mutate({ ...settings, ...patch });
  const editable = isAdmin && !save.isPending;

  return (
    <article
      className="rounded-xl border border-white/8 bg-white/2"
      data-native-device={device.id}
    >
      <header className="flex flex-wrap items-start gap-3 border-b border-white/6 px-4 py-3">
        <Speaker className="mt-0.5 h-5 w-5 shrink-0 text-zinc-400" />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold text-white">
            {device.name}
            {device.is_default ? (
              <span className="ml-2 text-xs font-normal text-zinc-500">
                System default
              </span>
            ) : null}
          </h3>
          <p className="truncate text-xs text-zinc-500">
            {driverLabels[device.driver]} · {device.detail}
          </p>
          <p className="mt-1 text-xs text-zinc-400">
            {deviceCapabilitySummary(device)}
            {dsdSupportSummary(device) ? ` · ${dsdSupportSummary(device)}` : ""}
          </p>
          {hardwareSummary(device) ? (
            <p
              className="mt-0.5 text-[11px] text-zinc-500"
              data-hardware-summary
            >
              {hardwareSummary(device)}
            </p>
          ) : null}
        </div>
        <button
          className={cn(
            "h-8 rounded-full border px-3 text-xs font-medium transition-colors disabled:opacity-40",
            selected
              ? "border-[#b26bff]/50 bg-[#b26bff]/15 text-[#d9b8ff]"
              : "border-white/10 text-zinc-200 hover:bg-white/8",
          )}
          disabled={!device.formats}
          onClick={onUse}
          type="button"
        >
          {selected ? "Playing here" : "Play here"}
        </button>
      </header>

      <div className="space-y-4 px-4 py-4">
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium text-zinc-200">Exclusive Mode</p>
            <p className="text-xs leading-5 text-zinc-500">
              {settings.exclusive
                ? device.id.startsWith("alsa-pcm:")
                  ? `${device.mode_label}: Parson turns ALSA's conversions off. This PCM comes from your ALSA configuration and may be a chain of plugins, so while it plays the signal path lists the chain libasound built and rates it.`
                  : `${device.mode_label}: no other program or system mixer touches the audio, and the device switches to each track's format.`
                : `${device.mode_label}: the system may convert or mix the audio, so the path is rated High quality at best.`}
            </p>
          </div>
          <Toggle
            checked={settings.exclusive}
            disabled={!editable || !device.exclusive_capable}
            label="Exclusive Mode"
            onChange={(exclusive) => change({ exclusive })}
          />
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <label className="grid gap-1.5" htmlFor={rateId}>
            <span className="text-xs font-medium text-zinc-300">
              Max sample rate (PCM)
            </span>
            <select
              className={selectClass}
              disabled={!editable || !device.formats}
              id={rateId}
              onChange={(event) =>
                change({
                  max_sample_rate: event.target.value
                    ? Number(event.target.value)
                    : null,
                })
              }
              value={settings.max_sample_rate ?? ""}
            >
              <option value="">No limit</option>
              {maxRateOptions(device).map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
          <label className="grid gap-1.5" htmlFor={depthId}>
            <span className="text-xs font-medium text-zinc-300">
              Max bits per sample
            </span>
            <select
              className={selectClass}
              disabled={!editable || !device.formats}
              id={depthId}
              onChange={(event) =>
                change({
                  max_bit_depth: event.target.value
                    ? Number(event.target.value)
                    : null,
                })
              }
              value={settings.max_bit_depth ?? ""}
            >
              <option value="">No limit</option>
              {maxDepthOptions(device).map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
        </div>

        {device.formats?.dsd_rates ? (
          <div className="grid gap-3 sm:grid-cols-2" data-dsd-settings>
            <label className="grid gap-1.5" htmlFor={strategyId}>
              <span className="text-xs font-medium text-zinc-300">
                DSD playback strategy
              </span>
              <select
                className={selectClass}
                disabled={!editable}
                id={strategyId}
                onChange={(event) =>
                  change({ dsd_strategy: event.target.value as DsdStrategy })
                }
                value={settings.dsd_strategy ?? "auto"}
              >
                {DSD_STRATEGIES.map((strategy) => (
                  <option key={strategy.value} value={strategy.value}>
                    {strategy.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="grid gap-1.5" htmlFor={dsdRateId}>
              <span className="text-xs font-medium text-zinc-300">
                Max DSD rate
              </span>
              <select
                className={selectClass}
                disabled={!editable || maxDsdRateOptions(device).length === 0}
                id={dsdRateId}
                onChange={(event) =>
                  change({
                    max_dsd_rate: event.target.value
                      ? Number(event.target.value)
                      : null,
                  })
                }
                value={settings.max_dsd_rate ?? ""}
              >
                <option value="">No limit</option>
                {maxDsdRateOptions(device).map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <p className="text-xs leading-5 text-zinc-500 sm:col-span-2">
              {dsdStrategyNote(device)}
            </p>
          </div>
        ) : null}

        <section aria-label="Bit-Perfect Format Support">
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">
            Bit-Perfect Format Support
          </h4>
          {device.formats ? (
            <>
              <ul className="flex flex-wrap gap-1.5" data-format-list="rates">
                {device.formats.sample_rates.map((entry) => (
                  <FormatChip
                    entry={entry}
                    key={entry.value}
                    label={nativeFormatLabel(entry.value)}
                  />
                ))}
              </ul>
              <ul
                className="mt-1.5 flex flex-wrap gap-1.5"
                data-format-list="depths"
              >
                {device.formats.bit_depths.map((entry) => (
                  <FormatChip
                    entry={entry}
                    key={entry.value}
                    label={`${entry.value} bit`}
                  />
                ))}
              </ul>
              {device.formats.dsd_rates ? (
                <ul
                  className="mt-1.5 flex flex-wrap gap-1.5"
                  data-format-list="dsd"
                >
                  {device.formats.dsd_rates.map((entry) => (
                    <FormatChip
                      entry={entry}
                      key={entry.value}
                      label={dsdRateLabel(entry.value)}
                    />
                  ))}
                </ul>
              ) : null}
              {descriptorLines(device).length ? (
                <div className="mt-3" data-usb-descriptors>
                  <p className="text-[11px] font-semibold uppercase tracking-wide text-zinc-500">
                    USB descriptors (from the kernel)
                  </p>
                  <ul className="mt-1 space-y-0.5 font-mono text-[11px] text-zinc-400">
                    {descriptorLines(device).map((line, index) => (
                      <li key={index}>{line}</li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <p className="mt-2 text-[11px] leading-4 text-zinc-500">
                Tracks in a format marked in red, or above your limits, are
                converted on the server to the closest supported format, and the
                signal path shows that conversion. Containers:{" "}
                {device.sample_formats.join(", ") || "none"}.
              </p>
            </>
          ) : (
            <p className="text-xs text-red-400">
              {device.probe_error ??
                "Parson could not read this device's formats."}
            </p>
          )}
        </section>
        {save.error ? (
          <p className="text-xs text-red-400">
            {save.error instanceof Error
              ? save.error.message
              : "Could not save the settings."}
          </p>
        ) : null}
      </div>
    </article>
  );
}

/** Settings → Audio outputs: the server's own devices. */
export default function AudioOutputSettings({ isAdmin }: { isAdmin: boolean }) {
  const outputs = useNativeOutputs();
  const queryClient = useQueryClient();
  const [choice, setChoice] = useOutputDeviceChoice();
  const refresh = useMutation({
    mutationFn: refreshNativeOutputs,
    onSuccess: (view) => queryClient.setQueryData(nativeOutputsKey, view),
  });
  const view = outputs.data;

  return (
    <div className="space-y-5" data-audio-output-settings>
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-semibold text-white">Audio outputs</h2>
          <p className="mt-1 max-w-xl text-sm leading-6 text-zinc-400">
            Devices attached to the computer running Parson. The server plays to
            them itself, opening the device exclusively so the file&apos;s
            samples reach the DAC unmodified. Browsers and phones can only play
            through the system mixer in shared mode.
          </p>
        </div>
        {isAdmin ? (
          <button
            className="flex h-9 items-center gap-2 rounded-full border border-white/10 px-3 text-xs text-zinc-200 hover:bg-white/8 disabled:opacity-50"
            disabled={refresh.isPending}
            onClick={() => refresh.mutate()}
            type="button"
          >
            <RefreshCw
              className={cn("h-3.5 w-3.5", refresh.isPending && "animate-spin")}
            />
            Find devices
          </button>
        ) : null}
      </div>

      {choice.startsWith(SERVER_OUTPUT_PREFIX) ? (
        <button
          className="text-xs text-zinc-400 underline underline-offset-2 hover:text-white"
          onClick={() => setChoice("")}
          type="button"
        >
          Play in this browser instead
        </button>
      ) : null}

      {outputs.isLoading ? (
        <p className="text-sm text-zinc-500">Looking for devices…</p>
      ) : !view ? (
        <p className="text-sm text-red-400">
          The server&apos;s outputs could not be loaded.
        </p>
      ) : !view.supported ? (
        <p className="rounded-xl border border-white/8 px-4 py-3 text-sm text-zinc-400">
          This server has no native audio output on this platform.
        </p>
      ) : view.devices.length === 0 ? (
        <div className="flex items-start gap-3 rounded-xl border border-dashed border-white/10 px-4 py-4 text-sm text-zinc-400">
          <HardDrive className="mt-0.5 h-5 w-5 shrink-0 text-zinc-500" />
          <p>
            No output devices were found on the server
            {view.driver ? ` (${driverLabels[view.driver]})` : ""}. Connect a
            DAC or sound card, then choose Find devices.
          </p>
        </div>
      ) : (
        <div className="space-y-4">
          {view.devices.map((device) => (
            <DeviceCard
              device={device}
              isAdmin={isAdmin}
              key={device.id}
              onUse={() => setChoice(`${SERVER_OUTPUT_PREFIX}${device.id}`)}
              selected={choice === `${SERVER_OUTPUT_PREFIX}${device.id}`}
            />
          ))}
        </div>
      )}
      {view?.simulated ? (
        <p className="text-[11px] leading-4 text-zinc-500">
          The simulated device is enabled (PARSON_NATIVE_OUTPUT_SIMULATE): it
          consumes audio in real time without a sound card, for testing.
        </p>
      ) : null}
      {!isAdmin ? (
        <p className="text-[11px] text-zinc-500">
          Only administrators can change device settings.
        </p>
      ) : null}
    </div>
  );
}
