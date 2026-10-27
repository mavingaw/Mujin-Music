"use client";

import {
  connectLink,
  getHostInfo,
  hostAddressLabel,
  hostOrigins,
  hostUptime,
  type HostInfo,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Check, Copy, Globe, Loader2, Smartphone, Wifi } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { toQR } from "toqr";
import { cn } from "@/lib/cn";
import {
  hostReachability,
  primaryHostOrigin,
  qrPath,
} from "@/features/server/host-presentation";

type QrTarget = "app" | "browser";

function QrCode({ value, label }: { value: string; label: string }) {
  const { size, path } = useMemo(() => {
    try {
      return qrPath(toQR(value));
    } catch {
      return { size: 0, path: "" };
    }
  }, [value]);
  if (!size) return null;
  const quiet = 3;
  return (
    <svg
      aria-label={label}
      className="h-full w-full"
      role="img"
      shapeRendering="crispEdges"
      viewBox={`${-quiet} ${-quiet} ${size + quiet * 2} ${size + quiet * 2}`}
    >
      <rect
        fill="#fff"
        height={size + quiet * 2}
        width={size + quiet * 2}
        x={-quiet}
        y={-quiet}
      />
      <path d={path} fill="#09090b" />
    </svg>
  );
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1_600);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <button
      aria-label={copied ? "Copied" : label}
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-zinc-500 transition-colors hover:bg-white/[0.07] hover:text-white"
      onClick={() => {
        void navigator.clipboard?.writeText(value).then(() => setCopied(true));
      }}
      title={copied ? "Copied" : label}
      type="button"
    >
      {copied ? (
        <Check className="h-4 w-4 text-emerald-400" />
      ) : (
        <Copy className="h-4 w-4" />
      )}
    </button>
  );
}

function ReachabilityNote({ info }: { info: HostInfo }) {
  const reachability = hostReachability(info);
  const copy = {
    ready: {
      tone: "good",
      text: "Visible on your network. Mujin Music apps on the same Wi-Fi find it automatically.",
    },
    "not-advertising": {
      tone: "warn",
      text: "Automatic discovery is off on this network, so other devices need the address or QR code.",
    },
    "no-network": {
      tone: "warn",
      text: "This computer is not connected to a network right now.",
    },
    "loopback-only": {
      tone: "warn",
      text: "This server only accepts connections from this computer. Set PARSON_BIND_ADDRESS=0.0.0.0 to let other devices connect.",
    },
  }[reachability.kind];
  return (
    <p
      className={cn(
        "flex items-start gap-2 text-sm leading-5",
        copy.tone === "good" ? "text-zinc-400" : "text-amber-200/90",
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          "mt-1.5 h-2 w-2 shrink-0 rounded-full",
          copy.tone === "good"
            ? "bg-emerald-400 shadow-[0_0_10px_rgba(163,201,139,0.6)]"
            : "bg-amber-400",
        )}
      />
      {copy.text}
    </p>
  );
}

export default function HostPanel() {
  const [target, setTarget] = useState<QrTarget>("app");
  const [pageOrigin, setPageOrigin] = useState("");
  useEffect(() => setPageOrigin(window.location.origin), []);
  const host = useQuery({
    queryKey: ["host-info"],
    queryFn: getHostInfo,
    staleTime: 30_000,
    refetchInterval: 60_000,
  });
  const info = host.data;
  const primary = info ? primaryHostOrigin(info, pageOrigin) : "";
  const others = info
    ? hostOrigins(info).filter((origin) => origin !== primary)
    : [];
  const link = info && primary ? connectLink(info, primary, target) : "";
  const reachable = info
    ? hostReachability(info).kind !== "loopback-only"
    : false;

  return (
    <section
      aria-labelledby="host-heading"
      className="overflow-hidden rounded-2xl border border-white/10 bg-[radial-gradient(120%_140%_at_0%_0%,rgba(255,255,255,0.06),transparent_55%)]"
    >
      <div className="grid gap-7 p-5 sm:p-6 md:grid-cols-[224px_minmax(0,1fr)]">
        <div className="flex flex-col items-center gap-3">
          <div className="relative aspect-square w-[224px] overflow-hidden rounded-2xl bg-white p-1 shadow-[0_20px_60px_rgba(0,0,0,0.55)] ring-1 ring-white/20">
            {host.isPending ? (
              <div className="flex h-full items-center justify-center bg-zinc-100">
                <Loader2 className="h-6 w-6 animate-spin text-zinc-400" />
              </div>
            ) : link && reachable ? (
              <QrCode
                label={`QR code that opens ${primary} in ${target === "app" ? "the Mujin Music app" : "a browser"}`}
                value={link}
              />
            ) : (
              <div className="flex h-full items-center justify-center bg-zinc-100 p-6 text-center text-xs font-medium text-zinc-500">
                No network address to share yet
              </div>
            )}
          </div>
          <div
            aria-label="What the QR code opens"
            className="grid w-[224px] grid-cols-2 rounded-full border border-white/10 bg-black/40 p-1 text-xs font-semibold"
            role="radiogroup"
          >
            {(
              [
                ["app", "Mujin Music app", Smartphone],
                ["browser", "Browser", Globe],
              ] as const
            ).map(([value, label, Icon]) => (
              <button
                aria-checked={target === value}
                className={cn(
                  "flex h-8 items-center justify-center gap-1.5 whitespace-nowrap rounded-full px-2 transition-colors",
                  target === value
                    ? "bg-white text-black"
                    : "text-zinc-400 hover:text-white",
                )}
                key={value}
                onClick={() => setTarget(value)}
                role="radio"
                type="button"
              >
                <Icon className="h-3.5 w-3.5" />
                {label}
              </button>
            ))}
          </div>
        </div>

        <div className="min-w-0">
          <p className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
            Server
          </p>
          <h2
            className="mt-1 truncate text-2xl font-semibold tracking-tight text-white"
            id="host-heading"
          >
            {info?.name ?? "This Mujin Music server"}
          </h2>
          <p className="mt-2 max-w-md text-sm leading-6 text-zinc-400">
            {target === "app"
              ? "Point your phone’s camera at the code to open this library in the Mujin Music app, already connected. Then approve it with the pairing code below."
              : "Point your phone’s camera at the code to open Mujin Music in its browser. Nothing to install."}
          </p>

          {host.isError ? (
            <p className="mt-5 text-sm text-zinc-500">
              Could not read this server’s network details.
            </p>
          ) : info ? (
            <>
              {primary ? (
                <div className="mt-5 flex items-center gap-2 rounded-xl border border-white/10 bg-black/40 py-1.5 pl-4 pr-1.5">
                  <Wifi className="h-4 w-4 shrink-0 text-zinc-500" />
                  <code className="min-w-0 flex-1 truncate font-mono text-[15px] text-white">
                    {primary}
                  </code>
                  <CopyButton label="Copy address" value={primary} />
                </div>
              ) : null}
              <div className="mt-4">
                <ReachabilityNote info={info} />
              </div>
              {others.length ? (
                <div className="mt-5">
                  <p className="mb-1.5 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
                    Other addresses
                  </p>
                  <ul className="divide-y divide-white/[0.06]">
                    {others.map((origin) => {
                      const address = info.addresses.find(
                        (candidate) => candidate.origin === origin,
                      );
                      return (
                        <li
                          className="flex items-center gap-3 py-1.5"
                          key={origin}
                        >
                          <code className="min-w-0 flex-1 truncate font-mono text-[13px] text-zinc-300">
                            {origin}
                          </code>
                          <span className="shrink-0 rounded-full border border-white/10 px-2 py-0.5 text-[11px] font-medium text-zinc-400">
                            {address ? hostAddressLabel(address) : "Public URL"}
                          </span>
                          <CopyButton label={`Copy ${origin}`} value={origin} />
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ) : null}
              <p className="mt-5 text-xs text-zinc-500">
                {[
                  info.serverVersion ? `Mujin Music ${info.serverVersion}` : "",
                  info.startedAt
                    ? `running for ${hostUptime(info.startedAt)}`
                    : "",
                  `port ${info.port}`,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
            </>
          ) : (
            <div className="mt-5 h-12 animate-pulse rounded-xl bg-white/[0.04]" />
          )}
        </div>
      </div>
    </section>
  );
}
