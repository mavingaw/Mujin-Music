"use client";

import {
  addNetworkPlayer,
  dsdRateLabel,
  DSD_STRATEGIES,
  formatSampleRate,
  getNetworkPlayers,
  networkOutputIdentity,
  networkProtocolLabels,
  outputErrorMessage,
  removeNetworkPlayer,
  type DsdStrategy,
  type FormatSupportEntry,
  type NetworkPlayers,
  type NetworkProtocol,
  type ZoneSettingsPatch,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  FlaskConical,
  HardDrive,
  Network,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { useId, useState } from "react";
import { OutputArtwork, ProtocolBadge } from "@/features/player/output-artwork";
import { errorText } from "@/features/player/zones/zone-settings-errors";
import { useZones } from "@/features/player/zones/zones-provider";
import { cn } from "@/lib/cn";
import {
  descriptorLines,
  deviceCapabilitySummary,
  dsdStrategyNote,
  dsdSupportSummary,
  formatTone,
  formatToneLabel,
  hardwareSummary,
  maxDepthOptions,
  maxDsdRateOptions,
  maxRateOptions,
  networkLimitsNote,
  networkStreamNote,
  outputEntries,
  type OutputEntry,
} from "./audio-outputs-model";

const networkPlayersKey = ["network-players"] as const;

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
  entry,
  isAdmin,
  selected,
  onUse,
}: {
  entry: OutputEntry;
  isAdmin: boolean;
  selected: boolean;
  onUse: () => void;
}) {
  const zones = useZones();
  const queryClient = useQueryClient();
  const rateId = useId();
  const depthId = useId();
  const strategyId = useId();
  const dsdRateId = useId();
  const { zone, endpoint: device } = entry;
  const save = useMutation({
    mutationFn: (patch: ZoneSettingsPatch) =>
      zones.updateSettings(zone.id, patch),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ["signal-path"] }),
  });
  const settings = zone.settings;
  const supports = settings?.supports;
  const exclusive = settings?.exclusive ?? false;
  const change = (patch: ZoneSettingsPatch) => save.mutate(patch);
  const editable = isAdmin && !save.isPending;
  const network = device.network ?? null;

  return (
    <article
      className={cn(
        "rounded-xl border bg-white/2",
        network ? "border-white/10" : "border-white/8",
      )}
      data-output-zone={zone.id}
      data-network-protocol={network?.protocol}
    >
      <header className="flex flex-wrap items-start gap-3 border-b border-white/6 px-4 py-3">
        <OutputArtwork
          driver={device.driver?.toLowerCase()}
          hasIcon={device.has_icon}
          kind={network?.kind ?? null}
          protocol={network?.protocol ?? null}
          size={network ? 52 : 40}
          zoneId={zone.id}
        />
        <div className="min-w-[12rem] flex-1">
          <h3 className="flex min-w-0 items-center gap-2 text-sm font-semibold text-white">
            <span className="truncate">{zone.name}</span>
            {network ? (
              <ProtocolBadge
                label={networkProtocolLabels[network.protocol]}
                protocol={network.protocol}
              />
            ) : null}
          </h3>
          <p className="truncate text-xs text-zinc-500">
            {network
              ? `${networkOutputIdentity(network)} · ${network.host}:${network.port}${network.manual ? " · added by address" : ""}`
              : [device.driver, device.detail].filter(Boolean).join(" · ")}
          </p>
          <p className="mt-1 text-xs text-zinc-400">
            {deviceCapabilitySummary(device)}
            {network
              ? " · DSD is converted to PCM on the server"
              : dsdSupportSummary(device)
                ? ` · ${dsdSupportSummary(device)}`
                : ""}
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
        {network ? (
          <div className="space-y-2" data-network-details>
            <p className="text-xs leading-5 text-zinc-400">
              {networkStreamNote(device)}
            </p>
            {network.accepts.length ? (
              <p className="text-[11px] text-zinc-500">
                The player lists:{" "}
                <span className="text-zinc-300">
                  {network.accepts.join(", ")}
                </span>
              </p>
            ) : null}
            <p
              className="flex items-start gap-2 rounded-lg border border-amber-400/20 bg-amber-400/5 px-3 py-2 text-[11px] leading-4 text-amber-200/90"
              data-network-verification
            >
              <FlaskConical className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              {network.verification}
            </p>
          </div>
        ) : null}
        <div
          className={cn(
            "flex items-start gap-3",
            !supports?.exclusive && "hidden",
          )}
        >
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium text-zinc-200">Exclusive Mode</p>
            <p className="text-xs leading-5 text-zinc-500">
              {exclusive
                ? device.id.includes("alsa-pcm:")
                  ? `${device.mode_label}: Parson turns ALSA's conversions off. This PCM comes from your ALSA configuration and may be a chain of plugins, so while it plays the signal path lists the chain libasound built and rates it.`
                  : `${device.mode_label}: no other program or system mixer touches the audio, and the device switches to each track's format.`
                : `${device.mode_label}: the system may convert or mix the audio, so the path is rated High quality at best.`}
            </p>
          </div>
          <Toggle
            checked={exclusive}
            disabled={!editable || !device.exclusive_capable}
            label="Exclusive Mode"
            onChange={(exclusive) => change({ exclusive })}
          />
        </div>

        <div
          className={cn(
            "grid gap-3 sm:grid-cols-2",
            !supports?.max_format && "hidden",
          )}
        >
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
              value={settings?.max_sample_rate ?? ""}
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
              value={settings?.max_bit_depth ?? ""}
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

        {device.formats?.dsd_rates && supports?.dsd ? (
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
                value={settings?.dsd_strategy ?? "auto"}
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
                value={settings?.max_dsd_rate ?? ""}
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
              {dsdStrategyNote(device, {
                exclusive,
                dsd_strategy: settings?.dsd_strategy,
                max_sample_rate: settings?.max_sample_rate,
                max_bit_depth: settings?.max_bit_depth,
              })}
            </p>
          </div>
        ) : null}

        <section aria-label="Bit-Perfect Format Support">
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">
            {network
              ? "Format limits for this player"
              : "Bit-Perfect Format Support"}
          </h4>
          {network ? (
            <p
              className="mb-2 text-[11px] leading-4 text-zinc-400"
              data-network-limits
            >
              {networkLimitsNote(device)}
            </p>
          ) : null}
          {device.formats ? (
            <>
              <ul className="flex flex-wrap gap-1.5" data-format-list="rates">
                {device.formats.sample_rates.map((entry) => (
                  <FormatChip
                    entry={entry}
                    key={entry.value}
                    label={formatSampleRate(entry.value)}
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
              {device.formats.dsd_rates && !network ? (
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
            {errorText(save.error, "Could not save the settings.")}
          </p>
        ) : null}
      </div>
    </article>
  );
}

const PROTOCOL_CHOICES: {
  value: NetworkProtocol;
  label: string;
  hint: string;
}[] = [
  {
    value: "open_home",
    label: "UPnP / OpenHome renderer",
    hint: "http://192.168.1.40:49152/description.xml, or just 192.168.1.40",
  },
  {
    value: "air_play",
    label: "AirPlay receiver",
    hint: "192.168.1.50 or 192.168.1.50:7000",
  },
];

/** Adds a network player by address, for networks discovery cannot reach. */
function AddNetworkPlayer() {
  const queryClient = useQueryClient();
  const [protocol, setProtocol] = useState<NetworkProtocol>("open_home");
  const [address, setAddress] = useState("");
  const [name, setName] = useState("");
  const protocolId = useId();
  const addressId = useId();
  const nameId = useId();
  const zones = useZones();
  const add = useMutation({
    mutationFn: () => addNetworkPlayer({ protocol, address, name }),
    onSuccess: (view) => {
      queryClient.setQueryData(networkPlayersKey, view);
      void zones.rescanOutputs().catch(() => {});
      setAddress("");
      setName("");
    },
  });
  const choice = PROTOCOL_CHOICES.find((item) => item.value === protocol)!;
  return (
    <form
      className="grid gap-2 rounded-xl border border-white/8 bg-white/2 px-4 py-3 sm:grid-cols-[minmax(0,13rem)_minmax(0,1fr)_minmax(0,9rem)_auto] sm:items-end"
      data-add-network-player
      onSubmit={(event) => {
        event.preventDefault();
        if (address.trim()) add.mutate();
      }}
    >
      <label className="grid gap-1" htmlFor={protocolId}>
        <span className="text-[11px] font-medium text-zinc-400">Protocol</span>
        <select
          className={selectClass}
          id={protocolId}
          onChange={(event) =>
            setProtocol(event.target.value as NetworkProtocol)
          }
          value={protocol}
        >
          {PROTOCOL_CHOICES.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </label>
      <label className="grid gap-1" htmlFor={addressId}>
        <span className="text-[11px] font-medium text-zinc-400">Address</span>
        <input
          className={cn(selectClass, "min-w-0")}
          id={addressId}
          onChange={(event) => setAddress(event.target.value)}
          placeholder={choice.hint}
          value={address}
        />
      </label>
      <label className="grid gap-1" htmlFor={nameId}>
        <span className="text-[11px] font-medium text-zinc-400">
          Name (AirPlay)
        </span>
        <input
          className={cn(selectClass, "min-w-0")}
          disabled={protocol !== "air_play"}
          id={nameId}
          onChange={(event) => setName(event.target.value)}
          placeholder="Kitchen"
          value={name}
        />
      </label>
      <button
        className="flex h-9 items-center justify-center gap-1.5 rounded-full bg-white px-4 text-xs font-semibold text-black hover:bg-zinc-200 disabled:opacity-50"
        disabled={add.isPending || !address.trim()}
        type="submit"
      >
        <Plus className={cn("h-3.5 w-3.5", add.isPending && "animate-spin")} />
        {add.isPending ? "Checking" : "Add player"}
      </button>
      {add.error ? (
        <p className="text-xs text-red-400 sm:col-span-4">
          {outputErrorMessage(add.error, "The player could not be added.")}
        </p>
      ) : null}
    </form>
  );
}

/** Players added by address, each with a remove button. */
function ManualPlayers({ players }: { players: NetworkPlayers }) {
  const queryClient = useQueryClient();
  const zones = useZones();
  const remove = useMutation({
    mutationFn: removeNetworkPlayer,
    onSuccess: (view) => {
      queryClient.setQueryData(networkPlayersKey, view);
      void zones.rescanOutputs().catch(() => {});
    },
  });
  if (!players.manual.length) return null;
  return (
    <ul className="space-y-1.5" data-manual-players>
      {players.manual.map((player) => (
        <li
          className="flex items-center gap-3 rounded-lg border border-white/8 px-3 py-2 text-xs text-zinc-300"
          key={player.address}
        >
          <ProtocolBadge
            label={networkProtocolLabels[player.protocol]}
            protocol={player.protocol}
          />
          <span className="min-w-0 flex-1 truncate">
            {player.name ? `${player.name} · ` : ""}
            {player.address}
          </span>
          <button
            aria-label={`Remove ${player.name ?? player.address}`}
            className="flex h-7 w-7 items-center justify-center rounded-full border border-white/10 text-zinc-400 hover:bg-white/8 hover:text-white disabled:opacity-40"
            disabled={remove.isPending}
            onClick={() => remove.mutate(player.address)}
            type="button"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </li>
      ))}
      {remove.error ? (
        <li className="text-xs text-red-400">
          {outputErrorMessage(remove.error, "The player could not be removed.")}
        </li>
      ) : null}
    </ul>
  );
}

/**
 * Settings → Audio outputs: the zones the server plays itself (its own
 * devices, bridges' devices and network players), with the physical
 * endpoint behind each. Their setup is the zone's settings.
 */
export default function AudioOutputSettings({ isAdmin }: { isAdmin: boolean }) {
  const zones = useZones();
  const players = useQuery({
    queryKey: networkPlayersKey,
    queryFn: getNetworkPlayers,
    staleTime: 30_000,
    retry: 1,
  });
  const refresh = useMutation({ mutationFn: zones.rescanOutputs });
  const { local, network } = outputEntries(zones.zones);
  const card = (entry: OutputEntry) => (
    <DeviceCard
      entry={entry}
      isAdmin={isAdmin}
      key={entry.zone.id}
      onUse={() => zones.selectZone(entry.zone.id)}
      selected={zones.selectedZoneId === entry.zone.id}
    />
  );
  const simulated = local.some(
    (entry) => entry.endpoint.driver?.toLowerCase() === "simulated",
  );

  return (
    <div className="space-y-5" data-audio-output-settings>
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-semibold text-white">Audio outputs</h2>
          <p className="mt-1 max-w-xl text-sm leading-6 text-zinc-400">
            Where the server plays itself: devices attached to the computer
            running Parson (and to Parson Bridges), opened exclusively so the
            file&apos;s samples reach the DAC unmodified, and network players it
            streams to after its own DSP. Each is a zone: pick it in the zone
            picker to play there. Browsers and phones can only play through the
            system mixer in shared mode.
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
            {refresh.isPending ? "Searching" : "Find devices"}
          </button>
        ) : null}
      </div>

      {zones.selectedZoneId && zones.selectedZoneId !== zones.localZoneId ? (
        <button
          className="text-xs text-zinc-400 underline underline-offset-2 hover:text-white"
          onClick={() => zones.selectZone(null)}
          type="button"
        >
          Play in this browser instead
        </button>
      ) : null}

      {zones.loading ? (
        <p className="text-sm text-zinc-500">
          Looking for devices and network players…
        </p>
      ) : !zones.loaded ? (
        <p className="text-sm text-red-400">
          The server&apos;s outputs could not be loaded.
        </p>
      ) : (
        <>
          <section aria-label="On this server" className="space-y-3">
            <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">
              <HardDrive className="h-3.5 w-3.5" /> On this server
            </h3>
            {local.length === 0 ? (
              <div className="flex items-start gap-3 rounded-xl border border-dashed border-white/10 px-4 py-4 text-sm text-zinc-400">
                <HardDrive className="mt-0.5 h-5 w-5 shrink-0 text-zinc-500" />
                <p>
                  No output devices were found on the server. Connect a DAC or
                  sound card, then choose Find devices.
                </p>
              </div>
            ) : (
              <div className="space-y-4">{local.map(card)}</div>
            )}
          </section>

          <section
            aria-label="On the network"
            className="space-y-3"
            data-network-outputs
          >
            <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-zinc-500">
              <Network className="h-3.5 w-3.5" /> On the network
            </h3>
            <p className="max-w-2xl text-xs leading-5 text-zinc-400">
              UPnP AV and OpenHome streamers and AirPlay receivers. The server
              runs the whole chain (decoder, DSP engine, any conversion the
              player needs) before the network hop, and each player keeps its
              own format limits.
              {players.data?.discovery === false
                ? " Discovery is off on this server (PARSON_NETWORK_DISCOVERY=0); add players by address."
                : " Parson searches with SSDP and mDNS when you choose Find devices."}
            </p>
            {network.length ? (
              <div className="space-y-4">{network.map(card)}</div>
            ) : (
              <div className="flex items-start gap-3 rounded-xl border border-dashed border-white/10 px-4 py-4 text-sm text-zinc-400">
                <Network className="mt-0.5 h-5 w-5 shrink-0 text-zinc-500" />
                <p>
                  No network players answered. If your network blocks multicast
                  (VLANs, some mesh Wi-Fi), add one by address.
                </p>
              </div>
            )}
            {isAdmin && players.data ? (
              <ManualPlayers players={players.data} />
            ) : null}
            {isAdmin ? <AddNetworkPlayer /> : null}
          </section>
        </>
      )}
      {simulated ? (
        <p className="text-[11px] leading-4 text-zinc-500">
          The simulated device is enabled (PARSON_NATIVE_OUTPUT_SIMULATE): it
          consumes audio in real time without a sound card, for testing.
        </p>
      ) : null}
      {!isAdmin ? (
        <p className="text-[11px] text-zinc-500">
          Only administrators can change device settings or add players.
        </p>
      ) : null}
    </div>
  );
}
