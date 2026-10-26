"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/cn";
import {
  checkRemoteAccess,
  getRemoteAccess,
  summarizeRemoteAccess,
  updateRemoteAccess,
  type CheckOutcome,
  type PathCheck,
  type RemoteAccessMode,
  type RemoteAccessStatus,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Globe,
  Loader2,
  Lock,
  Router,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { toast } from "sonner";
import { describeRouter, outcomeLabel } from "./remote-access-presentation";

const MODES: {
  value: RemoteAccessMode;
  label: string;
  icon: typeof Globe;
}[] = [
  { value: "off", label: "VPN only", icon: Lock },
  { value: "router", label: "Router", icon: Router },
  { value: "manual", label: "My address", icon: Globe },
];

function Light({ tone }: { tone: "good" | "warn" | "off" | "bad" }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "mt-1.5 h-2 w-2 shrink-0 rounded-full",
        tone === "good" &&
          "bg-emerald-400 shadow-[0_0_10px_rgba(52,211,153,0.7)]",
        tone === "warn" && "bg-amber-400",
        tone === "bad" && "bg-red-400",
        tone === "off" && "bg-zinc-600",
      )}
    />
  );
}

function outcomeTone(outcome: CheckOutcome) {
  return outcome === "reachable"
    ? "good"
    : outcome === "unreachable"
      ? "bad"
      : "warn";
}

function CheckRow({ result }: { result: PathCheck }) {
  return (
    <li className="flex items-start gap-3 py-2.5">
      <Light tone={outcomeTone(result.outcome)} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-sm font-medium text-zinc-100">
            {result.label}
          </span>
          <code className="min-w-0 truncate font-mono text-[12px] text-zinc-500">
            {result.origin}
          </code>
        </div>
        <p className="mt-0.5 text-xs leading-5 text-zinc-500">
          {result.detail}
        </p>
      </div>
      <span
        className={cn(
          "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium tabular-nums",
          result.outcome === "reachable"
            ? "bg-emerald-400/10 text-emerald-300"
            : "bg-white/[0.05] text-zinc-400",
        )}
      >
        {outcomeLabel(result)}
      </span>
    </li>
  );
}

/**
 * Settings → Connections → Away from home. Lets an administrator choose how
 * the library is reached outside the home network and proves each way works.
 */
export default function RemoteAccessPanel() {
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: ["remote-access"],
    queryFn: getRemoteAccess,
    retry: false,
    staleTime: 15_000,
  });
  const [busy, setBusy] = useState<"save" | "check" | null>(null);
  const [mode, setMode] = useState<RemoteAccessMode>("off");
  const [manualUrl, setManualUrl] = useState("");
  const [externalPort, setExternalPort] = useState("");
  const [error, setError] = useState<string | null>(null);
  const data = status.data;

  useEffect(() => {
    if (!data) return;
    setMode(data.config.mode);
    setManualUrl(data.config.manual_url ?? "");
    setExternalPort(
      data.config.external_port ? String(data.config.external_port) : "",
    );
  }, [data]);

  // Hidden for people who are not administrators (the endpoint says 403).
  if (status.isError) return null;

  function store(next: RemoteAccessStatus) {
    queryClient.setQueryData(["remote-access"], next);
    void queryClient.invalidateQueries({ queryKey: ["host-info"] });
  }

  async function save(nextMode: RemoteAccessMode, event?: FormEvent) {
    event?.preventDefault();
    if (busy) return;
    setBusy("save");
    setError(null);
    try {
      const port = externalPort.trim() ? Number(externalPort) : null;
      const next = await updateRemoteAccess({
        mode: nextMode,
        manual_url: manualUrl.trim() || null,
        external_port: port && Number.isInteger(port) ? port : null,
      });
      store(next);
      if (next.addresses.length) {
        setBusy("check");
        const report = await checkRemoteAccess();
        store({ ...next, last_check: report });
      }
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not save that change.",
      );
      setMode(data?.config.mode ?? "off");
    } finally {
      setBusy(null);
    }
  }

  async function runCheck() {
    if (busy || !data) return;
    setBusy("check");
    try {
      const report = await checkRemoteAccess();
      store({ ...data, last_check: report });
    } catch {
      toast("The check could not run. Try again.");
    } finally {
      setBusy(null);
    }
  }

  const summary = data ? summarizeRemoteAccess(data) : null;
  const vpn = data?.addresses.filter((address) => address.kind === "vpn");
  const router = data ? describeRouter(data.router) : null;

  return (
    <section
      aria-labelledby="remote-access-heading"
      className="overflow-hidden rounded-2xl border border-white/10 bg-white/[0.025]"
    >
      <div className="flex flex-col gap-4 p-5 sm:flex-row sm:items-start sm:p-6">
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
            Away from home
          </p>
          <h2
            className="mt-1 text-xl font-semibold tracking-tight text-white"
            id="remote-access-heading"
          >
            {summary?.title ?? "Remote access"}
          </h2>
          <p className="mt-1.5 max-w-xl text-sm leading-6 text-zinc-400">
            {summary?.detail ??
              "Listen to your library from anywhere, in full quality."}
          </p>
        </div>
        <Button
          className="shrink-0 rounded-full"
          disabled={!data || Boolean(busy) || !data.addresses.length}
          onClick={() => void runCheck()}
          size="sm"
          variant="outline"
        >
          {busy === "check" ? (
            <Loader2 className="animate-spin" />
          ) : (
            <ShieldCheck />
          )}
          {busy === "check" ? "Checking…" : "Check connection"}
        </Button>
      </div>

      {data?.last_check?.results.length ? (
        <ul className="mx-5 mb-2 divide-y divide-white/[0.06] border-y border-white/[0.06] sm:mx-6">
          {data.last_check.results.map((result) => (
            <CheckRow key={result.origin} result={result} />
          ))}
        </ul>
      ) : null}

      <div className="grid gap-5 border-t border-white/[0.06] p-5 sm:p-6">
        <div className="flex items-start gap-3">
          <Light tone={vpn?.length ? "good" : "off"} />
          <div className="min-w-0 text-sm">
            <p className="font-medium text-zinc-100">
              Tailscale or another VPN{" "}
              <span className="ml-1 rounded-full bg-white/[0.06] px-2 py-0.5 text-[11px] font-medium text-zinc-400">
                Recommended
              </span>
            </p>
            <p className="mt-0.5 leading-5 text-zinc-500">
              {vpn?.length
                ? `Found ${vpn.map((address) => address.origin.replace(/^https?:\/\//, "")).join(" and ")}. Phones signed in to the same VPN use it automatically. Nothing is open to the internet.`
                : "Install Tailscale on this server and on your phone. Nothing is opened to the internet, the connection is encrypted, and Parson finds the address on its own."}
            </p>
          </div>
        </div>

        <form
          className="grid gap-4"
          onSubmit={(event) => void save(mode, event)}
        >
          <div>
            <p className="text-sm font-medium text-zinc-100">
              Open it to the internet too
            </p>
            <div
              aria-label="How the library is reached from the internet"
              className="mt-2 grid max-w-md grid-cols-3 rounded-full border border-white/10 bg-black/40 p-1 text-xs font-semibold"
              role="radiogroup"
            >
              {MODES.map(({ value, label, icon: Icon }) => (
                <button
                  aria-checked={mode === value}
                  className={cn(
                    "flex h-8 items-center justify-center gap-1.5 whitespace-nowrap rounded-full px-2 transition-colors disabled:opacity-50",
                    mode === value
                      ? "bg-white text-black"
                      : "text-zinc-400 hover:text-white",
                  )}
                  disabled={!data || Boolean(busy)}
                  key={value}
                  onClick={() => {
                    setMode(value);
                    setError(null);
                    if (value !== "manual") void save(value);
                  }}
                  role="radio"
                  type="button"
                >
                  <Icon className="h-3.5 w-3.5" />
                  {label}
                </button>
              ))}
            </div>
          </div>

          {mode === "router" && data ? (
            <div className="grid gap-3 rounded-xl border border-white/[0.08] bg-black/30 p-4">
              <div className="flex items-start gap-3">
                <Light tone={router?.tone ?? "off"} />
                <div className="min-w-0 text-sm">
                  <p className="font-medium text-zinc-100">{router?.title}</p>
                  <p className="mt-0.5 leading-5 text-zinc-500">
                    {router?.detail}
                  </p>
                </div>
              </div>
              <label className="flex flex-wrap items-center gap-3 text-sm text-zinc-400">
                Outside port
                <Input
                  className="h-9 w-28 font-mono"
                  inputMode="numeric"
                  maxLength={5}
                  onChange={(event) =>
                    setExternalPort(event.target.value.replace(/\D/g, ""))
                  }
                  placeholder={String(
                    data.router.internal_address?.split(":").pop() ??
                      data.router.external_port ??
                      1993,
                  )}
                  value={externalPort}
                />
                <Button
                  disabled={Boolean(busy)}
                  size="sm"
                  type="submit"
                  variant="secondary"
                >
                  {busy === "save" ? "Applying…" : "Apply"}
                </Button>
              </label>
              <p className="flex gap-2 text-xs leading-5 text-amber-200/80">
                <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />A port
                forward uses plain HTTP, so sign-ins and music cross the
                internet unencrypted. Use it to get going; for every day, prefer
                Tailscale or your own HTTPS address.
              </p>
            </div>
          ) : null}

          {mode === "manual" ? (
            <div className="grid gap-2 rounded-xl border border-white/[0.08] bg-black/30 p-4">
              <label
                className="text-sm text-zinc-400"
                htmlFor="remote-manual-url"
              >
                The address your reverse proxy, dynamic DNS name or VPN gives
                this server
              </label>
              <div className="flex gap-2">
                <Input
                  autoComplete="off"
                  className="font-mono"
                  id="remote-manual-url"
                  onChange={(event) => setManualUrl(event.target.value)}
                  placeholder="https://music.example.com"
                  spellCheck={false}
                  value={manualUrl}
                />
                <Button
                  disabled={Boolean(busy) || !manualUrl.trim()}
                  type="submit"
                >
                  {busy === "save" ? "Saving…" : "Save"}
                </Button>
              </div>
            </div>
          ) : null}

          {error ? (
            <p className="text-sm text-red-300" role="alert">
              {error}
            </p>
          ) : null}
        </form>

        <p className="text-xs leading-5 text-zinc-500">
          The Parson app saves these addresses when it connects at home and
          switches to them by itself when your home network is out of reach. The
          check above runs from this server; the app runs it again from wherever
          you are.
        </p>
      </div>
    </section>
  );
}
