"use client";

import { Button } from "@/components/ui/button";
import {
  InputOTP,
  InputOTPGroup,
  InputOTPSeparator,
  InputOTPSlot,
} from "@/components/ui/input-otp";
import getBaseURL from "@/lib/api/server-url";
import {
  approveDevicePairing,
  HOME_ASSISTANT_REPOSITORY,
  homeAssistantAutomationExample,
  homeAssistantConnection,
  isLoopbackHost,
} from "@parson/music-sdk";
import { REGEXP_ONLY_DIGITS } from "input-otp";
import {
  ArrowLeftRight,
  AudioWaveform,
  Check,
  Copy,
  House,
  Library,
  Loader2,
  Power,
  Radio,
  Speaker,
  TriangleAlert,
  Volume2,
  Zap,
} from "lucide-react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { toast } from "sonner";

const FEATURES: { icon: ReactNode; title: string; text: string }[] = [
  {
    icon: <Speaker className="h-4 w-4" />,
    title: "Every zone",
    text: "Apps, Sonos rooms, AirPlay, UPnP, audio devices and linked groups each become a media player.",
  },
  {
    icon: <ArrowLeftRight className="h-4 w-4" />,
    title: "Group and transfer",
    text: "Join rooms, set grouped volume, and move the queue from one zone to another.",
  },
  {
    icon: <Library className="h-4 w-4" />,
    title: "Your whole library",
    text: "Browse albums, artists, playlists and genres, or play anything by search.",
  },
  {
    icon: <Radio className="h-4 w-4" />,
    title: "Radio and thumbs",
    text: "Start a station from any album, artist or genre; thumbs up or down teaches it.",
  },
  {
    icon: <Zap className="h-4 w-4" />,
    title: "Instant updates",
    text: "Changes are pushed the moment they happen. Nothing polls your server.",
  },
  {
    icon: <AudioWaveform className="h-4 w-4" />,
    title: "Signal path",
    text: "Format, output and a bit-perfect flag ride along as player attributes.",
  },
  {
    icon: <Power className="h-4 w-4" />,
    title: "Standby",
    text: "Turning off a device zone stops it and lets the device go; turning on wakes it.",
  },
  {
    icon: <Volume2 className="h-4 w-4" />,
    title: "Volume on your amplifier",
    text: "Set a zone's volume to be handled by Home Assistant: each change becomes an event an automation can send to an IR-controlled amplifier.",
  },
];

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1_600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <button
      aria-label={copied ? `${label} copied` : `Copy ${label}`}
      className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-zinc-400 transition-colors hover:bg-white/8 hover:text-white focus-visible:ring-2 focus-visible:ring-white/40 focus-visible:outline-none"
      onClick={() => {
        void navigator.clipboard
          ?.writeText(value)
          .then(() => setCopied(true))
          .catch(() => toast("Could not copy. Select the text instead."));
      }}
      type="button"
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-emerald-400" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </button>
  );
}

function Value({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-2 rounded-md border border-white/8 bg-black/20 py-1.5 pr-1.5 pl-3">
      <div className="min-w-0">
        <div className="text-[11px] tracking-wide text-zinc-500 uppercase">
          {label}
        </div>
        <div className="truncate font-mono text-sm text-zinc-100">{value}</div>
      </div>
      <CopyButton label={label} value={value} />
    </div>
  );
}

function Step({
  number,
  title,
  children,
}: {
  number: number;
  title: string;
  children: ReactNode;
}) {
  return (
    <li className="relative grid grid-cols-[28px_minmax(0,1fr)] gap-4">
      <span
        aria-hidden
        className="flex h-7 w-7 items-center justify-center rounded-full bg-white/8 text-xs font-semibold text-white ring-1 ring-white/10"
      >
        {number}
      </span>
      <div className="min-w-0 pb-1">
        <h3 className="text-sm font-medium text-zinc-100">{title}</h3>
        <div className="mt-1.5 space-y-3 text-sm leading-6 text-zinc-400">
          {children}
        </div>
      </div>
    </li>
  );
}

export default function HomeAssistantSettings() {
  const [origin, setOrigin] = useState("");
  const [code, setCode] = useState("");
  const [approving, setApproving] = useState(false);
  const [approved, setApproved] = useState(false);

  useEffect(() => {
    setOrigin(getBaseURL());
  }, []);

  const connection = origin ? homeAssistantConnection(origin) : null;
  const loopback = connection ? isLoopbackHost(connection.host) : false;

  async function approve(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const digits = code.replace(/\D/g, "");
    if (digits.length !== 6 || approving) return;
    setApproving(true);
    try {
      const response = await approveDevicePairing(digits);
      if (!response.status) {
        toast(
          response.message ||
            "That code is invalid or expired. Ask Home Assistant for a new one.",
        );
        return;
      }
      setApproved(true);
      setCode("");
      toast.success(
        `${response.deviceName || "Home Assistant"} can now control Mujin Music.`,
      );
    } finally {
      setApproving(false);
    }
  }

  return (
    <div className="space-y-8">
      <section className="overflow-hidden rounded-xl border border-white/10 bg-[radial-gradient(120%_140%_at_0%_0%,rgba(217,139,95,0.12),transparent_55%),radial-gradient(120%_140%_at_100%_100%,rgba(163,201,139,0.08),transparent_50%)] p-5 sm:p-6">
        <div className="flex items-start gap-4">
          <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-sky-400/15 text-sky-300 ring-1 ring-sky-300/20">
            <House className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <h2 className="text-lg font-semibold text-white">
              Mujin Music for Home Assistant
            </h2>
            <p className="mt-1 max-w-prose text-sm leading-6 text-zinc-400">
              Control every zone from dashboards, automations and voice
              assistants. Home Assistant talks to this server directly on your
              network, so music keeps its full quality and nothing goes through
              the cloud.
            </p>
          </div>
        </div>
      </section>

      <section aria-labelledby="ha-setup">
        <h2 id="ha-setup" className="mb-5 text-base font-semibold text-white">
          Set up
        </h2>
        <ol className="space-y-6">
          <Step number={1} title="Install the integration with HACS">
            <p>
              In HACS, open{" "}
              <strong className="text-zinc-200">⋮ → Custom repositories</strong>
              , add this repository as an <em>Integration</em>, then download{" "}
              <strong className="text-zinc-200">Mujin Music</strong> and restart
              Home Assistant.
            </p>
            <Value label="Repository" value={HOME_ASSISTANT_REPOSITORY} />
          </Step>
          <Step number={2} title="Let Home Assistant find this library">
            <p>
              It usually appears by itself under{" "}
              <strong className="text-zinc-200">
                Settings → Devices &amp; services
              </strong>
              . If it does not, add <em>Mujin Music</em> and enter:
            </p>
            {connection && (
              <div className="grid gap-2 sm:grid-cols-2">
                <Value label="Host" value={connection.host} />
                <Value label="Port" value={String(connection.port)} />
              </div>
            )}
            {loopback && (
              <p className="flex gap-2 rounded-md border border-amber-400/20 bg-amber-400/8 px-3 py-2 text-amber-200">
                <TriangleAlert className="mt-1 h-4 w-4 shrink-0" />
                You opened Mujin Music on this computer, so this is a local
                address. Use the computer&apos;s network address instead, and
                make sure network access is on in the server settings.
              </p>
            )}
          </Step>
          <Step number={3} title="Approve Home Assistant">
            <p>
              Choose <strong className="text-zinc-200">Pair with a code</strong>
              . Home Assistant shows six digits: enter them here. It gets its
              own session for your account, and no password is stored.
            </p>
            <form
              className="flex flex-wrap items-center gap-3"
              onSubmit={(event) => void approve(event)}
            >
              <InputOTP
                aria-label="Six-digit code from Home Assistant"
                autoComplete="one-time-code"
                disabled={approving}
                inputMode="numeric"
                maxLength={6}
                onChange={(value) => {
                  setApproved(false);
                  setCode(value.replace(/\D/g, "").slice(0, 6));
                }}
                pattern={REGEXP_ONLY_DIGITS}
                value={code}
              >
                <InputOTPGroup>
                  <InputOTPSlot index={0} />
                  <InputOTPSlot index={1} />
                  <InputOTPSlot index={2} />
                </InputOTPGroup>
                <InputOTPSeparator />
                <InputOTPGroup>
                  <InputOTPSlot index={3} />
                  <InputOTPSlot index={4} />
                  <InputOTPSlot index={5} />
                </InputOTPGroup>
              </InputOTP>
              <Button disabled={code.length !== 6 || approving} type="submit">
                {approving ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  "Approve"
                )}
              </Button>
              {approved && (
                <span
                  className="inline-flex items-center gap-1.5 text-sm text-emerald-400"
                  role="status"
                >
                  <Check className="h-4 w-4" />
                  Approved. Select Submit in Home Assistant.
                </span>
              )}
            </form>
          </Step>
        </ol>
      </section>

      <section
        aria-labelledby="ha-features"
        className="border-t border-white/8 pt-7"
      >
        <h2
          id="ha-features"
          className="mb-4 text-base font-semibold text-white"
        >
          What Home Assistant gets
        </h2>
        <ul className="grid gap-3 sm:grid-cols-2">
          {FEATURES.map((feature) => (
            <li
              className="flex gap-3 rounded-lg border border-white/8 bg-white/[0.02] p-3.5"
              key={feature.title}
            >
              <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-white/6 text-zinc-200">
                {feature.icon}
              </span>
              <div className="min-w-0">
                <div className="text-sm font-medium text-zinc-100">
                  {feature.title}
                </div>
                <p className="mt-0.5 text-[13px] leading-5 text-zinc-400">
                  {feature.text}
                </p>
              </div>
            </li>
          ))}
        </ul>
      </section>

      <section
        aria-labelledby="ha-example"
        className="border-t border-white/8 pt-7"
      >
        <div className="mb-3 flex items-center justify-between gap-3">
          <h2 id="ha-example" className="text-base font-semibold text-white">
            Example automation
          </h2>
          <CopyButton
            label="automation"
            value={homeAssistantAutomationExample()}
          />
        </div>
        <p className="mb-3 text-sm text-zinc-400">
          Morning radio at a gentle volume. Thumbs up and down, transfer, sleep
          timer and pause all are available as actions too.
        </p>
        <pre className="overflow-x-auto rounded-lg border border-white/8 bg-black/40 p-4 font-mono text-[12.5px] leading-5 text-zinc-300">
          <code>{homeAssistantAutomationExample()}</code>
        </pre>
      </section>
    </div>
  );
}
