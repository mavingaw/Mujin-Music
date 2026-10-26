"use client";

import {
  isNetworkDriver,
  nativeFormatLabel,
  networkOutputIdentity,
  networkProtocolLabels,
  type NativeOutputDevice,
} from "@parson/music-sdk";
import { Check, MonitorSpeaker, Router, Speaker } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import { useOutputDeviceChoice } from "./dsp-store";
import { OutputArtwork, ProtocolBadge } from "./output-artwork";
import { ActionButton } from "./player-controls";
import {
  SERVER_OUTPUT_PREFIX,
  serverOutputId,
  useNativeOutputs,
} from "./use-native-outputs";

/**
 * What a device will do with a hi-res file, in a few words: the light
 * colour it can reach and its format ceiling.
 */
export function outputCapabilityLine(device: NativeOutputDevice): {
  text: string;
  tone: "lossless" | "high_quality" | "unknown";
} {
  if (device.probe_error && !device.formats)
    return { text: device.probe_error, tone: "unknown" };
  const rates = (device.formats?.sample_rates ?? []).filter(
    (entry) => entry.supported && entry.allowed,
  );
  const depths = (device.formats?.bit_depths ?? [])
    .filter((entry) => entry.supported && entry.allowed)
    .map((entry) => entry.value);
  if (!rates.length || !depths.length)
    return { text: "Formats unknown", tone: "unknown" };
  const ceiling = `${nativeFormatLabel(rates.at(-1)!.value)}/${Math.max(...depths)}bit`;
  if (device.network?.protocol === "air_play")
    return {
      text: `High quality: AirPlay carries ${ceiling}, the receiver sets the volume`,
      tone: "high_quality",
    };
  if (device.network)
    return {
      text: `Bit-perfect up to ${ceiling}${device.network.caps_source === "assumed" ? " (assumed)" : ""}, DSP on the server`,
      tone: "lossless",
    };
  return device.settings.exclusive
    ? { text: `Bit-perfect up to ${ceiling}, exclusive`, tone: "lossless" }
    : {
        text: `Shared mode up to ${ceiling}: high quality`,
        tone: "high_quality",
      };
}

const TONE_COLOR = {
  lossless: "text-[#c9a2ff]",
  high_quality: "text-emerald-300/90",
  unknown: "text-zinc-500",
} as const;

function OutputRow({
  active,
  device,
  onSelect,
}: {
  active: boolean;
  device: NativeOutputDevice;
  onSelect: () => void;
}) {
  const network = device.network ?? null;
  const line = outputCapabilityLine(device);
  return (
    <button
      className={cn(
        "flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-45",
        active
          ? "border-[#b26bff]/45 bg-[#b26bff]/10"
          : "border-white/8 hover:bg-white/5",
      )}
      data-output-choice={device.id}
      disabled={!device.formats}
      onClick={onSelect}
      title={device.probe_error ?? undefined}
      type="button"
    >
      <OutputArtwork
        deviceId={device.id}
        driver={device.driver}
        hasIcon={Boolean(network?.icon_url)}
        kind={network?.kind ?? null}
        protocol={network?.protocol ?? null}
        size={42}
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2 text-sm font-medium text-white">
          <span className="truncate">{device.name}</span>
          {network ? (
            <ProtocolBadge
              label={networkProtocolLabels[network.protocol]}
              protocol={network.protocol}
            />
          ) : null}
        </span>
        <span className="block truncate text-xs text-zinc-500">
          {network ? networkOutputIdentity(network) : device.mode_label}
        </span>
        <span
          className={cn("block truncate text-[11px]", TONE_COLOR[line.tone])}
        >
          {line.text}
        </span>
      </span>
      {active ? <Check className="h-4 w-4 shrink-0 text-[#c9a2ff]" /> : null}
    </button>
  );
}

/**
 * The player bar's output button: this browser, the server's own devices
 * or a network player (UPnP, OpenHome, AirPlay), as Roon's zone picker.
 */
export function OutputPickerButton() {
  const [open, setOpen] = useState(false);
  const outputs = useNativeOutputs();
  const [choice, setChoice] = useOutputDeviceChoice();
  const devices = outputs.data?.devices ?? [];
  const selectedId = serverOutputId(choice);
  const selected = devices.find((device) => device.id === selectedId) ?? null;
  const local = devices.filter((device) => !isNetworkDriver(device.driver));
  const network = devices.filter((device) => isNetworkDriver(device.driver));
  const select = (value: string) => {
    setChoice(value);
    setOpen(false);
  };
  const Icon = selected?.network ? Router : selected ? Speaker : MonitorSpeaker;
  return (
    <>
      <ActionButton
        active={Boolean(selected)}
        label={selected ? `Output: ${selected.name}` : "Output: this browser"}
        onClick={() => setOpen(true)}
      >
        <Icon className="h-4 w-4" />
      </ActionButton>
      <Dialog onOpenChange={setOpen} open={open}>
        <DialogContent className="max-w-md gap-0 p-0" data-output-picker>
          <header className="border-b border-white/6 px-5 pb-3 pt-5 pr-14">
            <DialogTitle className="text-lg font-semibold">Play to</DialogTitle>
            <DialogDescription className="mt-0.5 text-xs leading-5 text-zinc-500">
              The server plays its own devices and network players itself, after
              its DSP engine; this browser only reaches the system mixer.
            </DialogDescription>
          </header>
          <div className="max-h-[65vh] space-y-4 overflow-y-auto px-5 py-4">
            <button
              className={cn(
                "flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition-colors",
                !selected
                  ? "border-[#b26bff]/45 bg-[#b26bff]/10"
                  : "border-white/8 hover:bg-white/5",
              )}
              data-output-choice="browser"
              onClick={() => select("")}
              type="button"
            >
              <span className="inline-flex h-[42px] w-[42px] items-center justify-center rounded-xl bg-white/6 text-zinc-300 ring-1 ring-white/10">
                <MonitorSpeaker className="h-5 w-5" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium text-white">
                  This browser
                </span>
                <span className="block text-[11px] text-emerald-300/90">
                  High quality: the system mixer, shared mode
                </span>
              </span>
              {!selected ? (
                <Check className="h-4 w-4 shrink-0 text-[#c9a2ff]" />
              ) : null}
            </button>
            {local.length ? (
              <section className="space-y-1.5">
                <h3 className="text-[11px] font-semibold uppercase tracking-wide text-zinc-500">
                  On the server
                </h3>
                {local.map((device) => (
                  <OutputRow
                    active={device.id === selectedId}
                    device={device}
                    key={device.id}
                    onSelect={() =>
                      select(`${SERVER_OUTPUT_PREFIX}${device.id}`)
                    }
                  />
                ))}
              </section>
            ) : null}
            <section className="space-y-1.5">
              <h3 className="text-[11px] font-semibold uppercase tracking-wide text-zinc-500">
                On the network
              </h3>
              {network.length ? (
                network.map((device) => (
                  <OutputRow
                    active={device.id === selectedId}
                    device={device}
                    key={device.id}
                    onSelect={() =>
                      select(`${SERVER_OUTPUT_PREFIX}${device.id}`)
                    }
                  />
                ))
              ) : (
                <p className="rounded-xl border border-dashed border-white/10 px-3 py-3 text-xs text-zinc-500">
                  {outputs.isLoading
                    ? "Searching for streamers and AirPlay receivers…"
                    : "No network players found. Search again or add one by address in Settings."}
                </p>
              )}
            </section>
          </div>
          <footer className="border-t border-white/6 px-5 py-3 text-[11px] text-zinc-500">
            <Link
              className="text-zinc-300 underline underline-offset-2 hover:text-white"
              href="/settings?tab=outputs"
              onClick={() => setOpen(false)}
            >
              Settings → Audio outputs
            </Link>{" "}
            for format limits, Exclusive Mode and adding players by address.
          </footer>
        </DialogContent>
      </Dialog>
    </>
  );
}
