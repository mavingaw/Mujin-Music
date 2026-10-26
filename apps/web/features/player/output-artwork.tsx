"use client";

import {
  nativeOutputIconUrl,
  type NativeOutputDriver,
  type NetworkEndpointKind,
  type NetworkProtocol,
} from "@parson/music-sdk";
import {
  Cpu,
  HardDrive,
  Laptop,
  Radio,
  Router,
  Speaker,
  Tv,
  type LucideIcon,
} from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/cn";

const KIND_ICONS: Record<NetworkEndpointKind, LucideIcon> = {
  streamer: Router,
  speaker: Speaker,
  television: Tv,
  receiver: Radio,
  computer: Laptop,
};

/** Protocol tint, so a glance tells UPnP, OpenHome and AirPlay apart. */
const PROTOCOL_TINT: Record<NetworkProtocol, string> = {
  upnp_av: "from-sky-500/25 to-sky-500/5 text-sky-200",
  open_home: "from-violet-500/30 to-violet-500/5 text-violet-200",
  air_play: "from-zinc-200/20 to-zinc-200/5 text-zinc-100",
};

/**
 * A device's picture: a network player's own icon (UPnP), else a glyph
 * for its kind tinted by protocol; a USB/sound-card glyph for a device on
 * the server itself.
 */
export function OutputArtwork({
  className,
  deviceId,
  driver,
  hasIcon,
  kind,
  protocol,
  size = 44,
}: {
  className?: string;
  deviceId?: string | null;
  driver?: NativeOutputDriver | null;
  hasIcon?: boolean;
  kind?: NetworkEndpointKind | null;
  protocol?: NetworkProtocol | null;
  size?: number;
}) {
  const [failed, setFailed] = useState(false);
  const Icon = kind
    ? KIND_ICONS[kind]
    : driver === "simulated"
      ? Cpu
      : HardDrive;
  const tint = protocol
    ? PROTOCOL_TINT[protocol]
    : "from-white/12 to-white/3 text-zinc-300";
  const glyph = Math.round(size * 0.46);
  return (
    <span
      aria-hidden
      className={cn(
        "relative inline-flex shrink-0 items-center justify-center overflow-hidden rounded-xl bg-linear-to-br ring-1 ring-white/10",
        tint,
        className,
      )}
      data-output-artwork={hasIcon && !failed ? "icon" : (kind ?? "local")}
      style={{ height: size, width: size }}
    >
      {hasIcon && deviceId && !failed ? (
        <img
          alt=""
          className="h-full w-full object-contain p-1"
          onError={() => setFailed(true)}
          src={nativeOutputIconUrl(deviceId)}
        />
      ) : (
        <Icon style={{ height: glyph, width: glyph }} />
      )}
    </span>
  );
}

/** `UPnP AV`, `OpenHome`, `AirPlay` as a small pill. */
export function ProtocolBadge({
  label,
  protocol,
}: {
  label: string;
  protocol: NetworkProtocol;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center rounded-full border px-2 text-[10px] font-semibold uppercase tracking-wide",
        protocol === "open_home" &&
          "border-violet-400/35 bg-violet-500/10 text-violet-200",
        protocol === "upnp_av" &&
          "border-sky-400/35 bg-sky-500/10 text-sky-200",
        protocol === "air_play" &&
          "border-zinc-300/30 bg-zinc-200/8 text-zinc-200",
      )}
      data-protocol={protocol}
    >
      {label}
    </span>
  );
}
