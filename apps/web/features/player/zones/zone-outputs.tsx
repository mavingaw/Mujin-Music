"use client";

import { AudioWaveform, Cast, Plus, Radio, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { useSession } from "@/features/account/session-provider";
import {
  addBridgeError,
  bridgeSummary,
  outputCounts,
} from "./zone-outputs-model";
import { useZones } from "./zones-provider";

const TONE: Record<"ok" | "warn" | "off", string> = {
  ok: "bg-emerald-400 shadow-[0_0_8px_rgba(163,201,139,0.6)]",
  warn: "bg-amber-400",
  off: "bg-zinc-600",
};

/**
 * Parson Bridges, AirPlay receivers and Chromecasts the server plays to.
 * A bridge is another computer (a Raspberry Pi by the hi-fi, a desktop with
 * a DAC) running `parson-bridge`; each of its outputs becomes a zone.
 */
export function ZoneOutputsPanel() {
  const { outputs, addBridge, removeBridge, busy } = useZones();
  const { session } = useSession();
  const [adding, setAdding] = useState(false);
  const [address, setAddress] = useState("");
  const bridges = outputs?.bridges;
  if (
    !bridges?.enabled &&
    !outputs?.airplay?.enabled &&
    !outputs?.chromecast?.enabled
  ) {
    return null;
  }
  const counts = outputCounts(outputs);
  const isAdmin = session?.role === "admin";

  const submit = async () => {
    const value = address.trim();
    if (!value) return;
    try {
      const status = await addBridge(value);
      if (status.connected) {
        toast.success(
          `Connected to ${status.name ?? value}: ${status.devices} ${status.devices === 1 ? "output" : "outputs"}`,
        );
        setAddress("");
        setAdding(false);
      } else {
        toast.error(
          status.last_error
            ? `Saved, but not connected yet: ${status.last_error}`
            : "Saved. Mujin Music keeps trying to connect.",
        );
      }
    } catch (error) {
      toast.error(addBridgeError(error));
    }
  };

  return (
    <section
      aria-label="Bridges and network speakers"
      className="space-y-2 rounded-xl border border-white/8 bg-white/[0.03] p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-zinc-400">
          Bridges and network speakers
        </p>
        <div className="flex items-center gap-1.5 text-[11px] text-zinc-500">
          {outputs?.airplay?.enabled && (
            <span className="flex items-center gap-1 rounded-full bg-white/5 px-2 py-0.5">
              <Radio aria-hidden className="h-3 w-3" />
              AirPlay {counts.airplay}
            </span>
          )}
          {outputs?.chromecast?.enabled && (
            <span className="flex items-center gap-1 rounded-full bg-white/5 px-2 py-0.5">
              <Cast aria-hidden className="h-3 w-3" />
              Cast {counts.chromecast}
            </span>
          )}
        </div>
      </div>
      {bridges?.enabled && (
        <ul className="space-y-1">
          {bridges.bridges.map((bridge) => {
            const summary = bridgeSummary(bridge);
            return (
              <li
                className="group/bridge flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-white/[0.04]"
                key={bridge.address}
              >
                <AudioWaveform
                  aria-hidden
                  className="h-4 w-4 shrink-0 text-zinc-400"
                />
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-1.5 truncate text-xs font-medium text-zinc-200">
                    <span
                      aria-hidden
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${TONE[summary.tone]}`}
                    />
                    {summary.title}
                  </p>
                  <p
                    className="truncate text-[11px] text-zinc-500"
                    title={summary.detail}
                  >
                    {summary.detail}
                  </p>
                </div>
                {isAdmin && bridge.source === "manual" && (
                  <button
                    aria-label={`Forget ${summary.title}`}
                    className="rounded-full p-1 text-zinc-500 opacity-0 hover:bg-white/10 hover:text-white focus-visible:opacity-100 group-hover/bridge:opacity-100"
                    onClick={() =>
                      void removeBridge(bridge.address).catch(() =>
                        toast.error("Could not forget that bridge."),
                      )
                    }
                    type="button"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
              </li>
            );
          })}
          {!bridges.bridges.length && (
            <li className="px-2 text-[11px] leading-4 text-zinc-500">
              Run{" "}
              <code className="rounded bg-white/10 px-1 text-zinc-300">
                parson-bridge
              </code>{" "}
              on the computer your DAC is plugged into (Linux, including
              Raspberry Pi OS). It shows up here by itself, and each of its
              outputs becomes a zone with exclusive, bit-perfect playback.
            </li>
          )}
        </ul>
      )}
      {bridges?.enabled &&
        isAdmin &&
        (adding ? (
          <form
            className="flex items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              void submit();
            }}
          >
            <Input
              aria-label="Bridge address"
              autoFocus
              className="h-8 text-xs"
              onChange={(event) => setAddress(event.target.value)}
              placeholder="pi.local or 192.168.1.20:1994"
              value={address}
            />
            <button
              className="h-8 shrink-0 rounded-full bg-white px-3 text-xs font-semibold text-black disabled:opacity-40"
              disabled={busy || !address.trim()}
              type="submit"
            >
              Connect
            </button>
          </form>
        ) : (
          <button
            className="flex items-center gap-1.5 rounded-full px-2 py-1 text-[11px] text-zinc-400 hover:bg-white/10 hover:text-white"
            onClick={() => setAdding(true)}
            type="button"
          >
            <Plus className="h-3 w-3" />
            Add a bridge by address
          </button>
        ))}
      {counts.airplayUnsupported.length > 0 && (
        <p className="px-2 text-[11px] leading-4 text-zinc-500">
          {counts.airplayUnsupported.join(", ")}{" "}
          {counts.airplayUnsupported.length === 1 ? "speaks" : "speak"} only
          AirPlay 2 with HomeKit pairing, which Mujin Music does not do yet.
        </p>
      )}
      <p className="px-2 text-[11px] leading-4 text-zinc-600">
        Bridges run on Linux (ALSA), Windows (WASAPI exclusive) and macOS
        (CoreAudio hog mode). Bridges, AirPlay and server-side Chromecast are
        tested against simulated DACs and protocol-level fakes of each audio
        stack and device, not yet on real hardware.
      </p>
    </section>
  );
}
