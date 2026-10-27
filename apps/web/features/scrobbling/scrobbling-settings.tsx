"use client";

import {
  authorizeLastfm,
  clearLastfmServerCredentials,
  connectListenbrainz,
  disconnectScrobbling,
  getScrobbling,
  importScrobbleHistory,
  plausibleLastfmCredential,
  plausibleListenbrainzToken,
  retryScrobbles,
  scrobbleServiceSummary,
  setLastfmServerCredentials,
  updateScrobbleSettings,
  type ScrobbleService,
  type ScrobbleServiceStatus,
  type ScrobblingStatus,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight,
  CheckCircle2,
  Clock3,
  CloudOff,
  DownloadCloud,
  FlaskConical,
  KeyRound,
  Loader2,
  RotateCw,
  Unplug,
  XCircle,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useSession } from "@/features/account/session-provider";
import { RadioSwitch } from "@/features/player/radio/radio-controls";
import { cn } from "@/lib/cn";
import { hasDesktopBridge } from "@/lib/desktop/bridge";
import {
  activityLabel,
  importProgressLabel,
  listenTime,
} from "./scrobbling-model";

const scrobblingKey = ["scrobbling"] as const;

function errorMessage(error: unknown, fallback: string): string {
  const data = (error as { response?: { data?: { message?: unknown } } })
    ?.response?.data;
  return typeof data?.message === "string" ? data.message : fallback;
}

export function ServiceMark({
  service,
  size = 40,
}: {
  service: ScrobbleService;
  size?: number;
}) {
  return (
    <span
      aria-hidden="true"
      className="inline-flex shrink-0 items-center justify-center rounded-xl font-bold tracking-tight text-white shadow-[inset_0_0_0_1px_rgba(255,255,255,0.14)]"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.36),
        background:
          service === "lastfm"
            ? "linear-gradient(145deg, #ff3b30 0%, #b90000 100%)"
            : "linear-gradient(145deg, #eb743b 0%, #353070 100%)",
      }}
    >
      {service === "lastfm" ? "fm" : "LB"}
    </span>
  );
}

const toneStyles = {
  ok: "bg-emerald-400/12 text-emerald-300",
  warn: "bg-amber-400/12 text-amber-300",
  error: "bg-rose-400/12 text-rose-300",
  idle: "bg-white/6 text-zinc-400",
} as const;

function Toggle({
  checked,
  description,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  description: string;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-4 py-3",
        disabled && "opacity-50",
      )}
    >
      <div>
        <p className="text-sm text-zinc-200">{label}</p>
        <p className="mt-0.5 text-xs leading-5 text-zinc-500">{description}</p>
      </div>
      <RadioSwitch
        checked={checked}
        label={label}
        onChange={(next) => !disabled && onChange(next)}
      />
    </div>
  );
}

function ServiceCard({
  connecting,
  onConnectLastfm,
  onCancelConnect,
  service,
}: {
  connecting: boolean;
  onConnectLastfm: () => void;
  onCancelConnect: () => void;
  service: ScrobbleServiceStatus;
}) {
  const queryClient = useQueryClient();
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const summary = scrobbleServiceSummary(service);
  const connection = service.connection;
  const importing = service.import?.status === "running";

  const run = async (label: string, action: () => Promise<unknown>) => {
    setBusy(label);
    try {
      await action();
      await queryClient.invalidateQueries({ queryKey: scrobblingKey });
    } catch (cause) {
      toast.error(errorMessage(cause, "That did not work. Try again."));
    } finally {
      setBusy(null);
    }
  };

  const setting = (patch: Parameters<typeof updateScrobbleSettings>[1]) =>
    run("settings", async () => {
      const next = await updateScrobbleSettings(service.service, patch);
      queryClient.setQueryData(scrobblingKey, next);
    });

  const connectToken = (event: FormEvent) => {
    event.preventDefault();
    if (!plausibleListenbrainzToken(token)) return;
    void run("connect", async () => {
      const result = await connectListenbrainz(token);
      setToken("");
      toast.success(`ListenBrainz connected as ${result.accountName}`);
    });
  };

  return (
    <article
      className={cn(
        "flex flex-col rounded-2xl border bg-white/[0.02] p-5 transition-colors",
        connection?.status === "reauth_required"
          ? "border-rose-400/30"
          : connection
            ? "border-white/10"
            : "border-white/6",
      )}
    >
      <header className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <ServiceMark service={service.service} />
        <div className="min-w-0 flex-1 basis-40">
          <h3 className="text-sm font-semibold text-white">{service.name}</h3>
          {connection ? (
            <a
              className="mt-0.5 inline-flex items-center gap-1 text-xs text-zinc-400 hover:text-white"
              href={connection.profileUrl}
              rel="noreferrer"
              target="_blank"
            >
              {connection.accountName}
              <ArrowUpRight className="h-3 w-3" />
            </a>
          ) : (
            <p className="mt-0.5 text-xs text-zinc-500">
              {service.service === "lastfm"
                ? "Scrobble to your Last.fm profile."
                : "Open listening data, part of MetaBrainz."}
            </p>
          )}
        </div>
        <span
          className={cn(
            "ml-auto shrink-0 whitespace-nowrap rounded-full px-2 py-1 text-[11px] font-medium",
            toneStyles[summary.tone],
          )}
        >
          {summary.text}
        </span>
      </header>

      {connection?.lastError && (
        <p className="mt-3 flex items-start gap-2 rounded-lg bg-white/[0.03] px-3 py-2 text-xs leading-5 text-zinc-400">
          <CloudOff className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-300" />
          <span>
            {connection.lastError}
            {connection.status === "ok"
              ? " Listens are kept and sent automatically when it works again."
              : ` ${service.name} needs you to connect again. Listens are kept and sent once you do.`}
          </span>
        </p>
      )}

      {!connection || connection.status === "reauth_required" ? (
        <div className="mt-4">
          {service.service === "lastfm" ? (
            connecting ? (
              <div className="flex items-center justify-between gap-3 rounded-xl border border-white/8 bg-black/40 px-3 py-2.5">
                <p className="flex items-center gap-2 text-xs text-zinc-300">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Approve Parson in the Last.fm window…
                </p>
                <Button onClick={onCancelConnect} size="sm" variant="ghost">
                  Cancel
                </Button>
              </div>
            ) : (
              <Button
                className="w-full"
                disabled={!service.available}
                onClick={onConnectLastfm}
              >
                {connection ? "Connect again" : "Connect Last.fm"}
              </Button>
            )
          ) : (
            <form className="grid gap-2" onSubmit={connectToken}>
              <Input
                aria-label="ListenBrainz user token"
                autoComplete="off"
                onChange={(event) => setToken(event.target.value)}
                placeholder="User token from listenbrainz.org/settings"
                spellCheck={false}
                value={token}
              />
              <div className="flex items-center justify-between gap-2">
                <a
                  className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-200"
                  href="https://listenbrainz.org/settings/"
                  rel="noreferrer"
                  target="_blank"
                >
                  Find your token <ArrowUpRight className="h-3 w-3" />
                </a>
                <Button
                  disabled={!plausibleListenbrainzToken(token) || busy !== null}
                  size="sm"
                  type="submit"
                >
                  {busy === "connect" && <Loader2 className="animate-spin" />}
                  {connection ? "Reconnect" : "Connect"}
                </Button>
              </div>
            </form>
          )}
          {!service.available && (
            <p className="mt-2 text-xs leading-5 text-zinc-500">
              Last.fm needs an API account for this server. An administrator
              adds its key below.
            </p>
          )}
        </div>
      ) : (
        <>
          <div className="mt-2 divide-y divide-white/6">
            <Toggle
              checked={connection.enabled}
              description={
                service.service === "lastfm"
                  ? "Send what this profile plays. Off pauses sending: listens wait here and go out when you turn it back on (Last.fm takes listens up to 14 days old)."
                  : "Send what this profile plays. Off pauses sending: listens wait here and go out when you turn it back on."
              }
              label="Scrobble"
              onChange={(enabled) => void setting({ enabled })}
            />
            <Toggle
              checked={connection.sendNowPlaying}
              description="Show the current track on your profile while it plays."
              disabled={!connection.enabled}
              label="Now playing"
              onChange={(sendNowPlaying) => void setting({ sendNowPlaying })}
            />
            <Toggle
              checked={connection.scrobbleRadio}
              description="Include tracks Radio picked, marked as not chosen by you."
              disabled={!connection.enabled}
              label="Radio picks"
              onChange={(scrobbleRadio) => void setting({ scrobbleRadio })}
            />
          </div>
          <dl className="mt-3 grid grid-cols-3 gap-2 text-center">
            {[
              ["Sent", service.queue.sent],
              ["Waiting", service.queue.pending],
              ["Not taken", service.queue.ignored + service.queue.failed],
            ].map(([label, value]) => (
              <div className="rounded-xl bg-white/[0.03] px-2 py-2" key={label}>
                <dd className="text-base font-semibold tabular-nums text-white">
                  {value}
                </dd>
                <dt className="text-[10px] uppercase tracking-wide text-zinc-500">
                  {label}
                </dt>
              </div>
            ))}
          </dl>
          {service.import && (
            <div className="mt-3 rounded-xl bg-white/[0.03] px-3 py-2.5">
              <p className="flex items-center gap-2 text-xs text-zinc-300">
                {importing ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : service.import.status === "done" ? (
                  <CheckCircle2 className="h-3.5 w-3.5 text-emerald-300" />
                ) : (
                  <XCircle className="h-3.5 w-3.5 text-rose-300" />
                )}
                {importProgressLabel(service.import)}
              </p>
              {importing && (
                <div className="mt-2 h-1 overflow-hidden rounded-full bg-white/8">
                  <div className="h-full w-1/3 animate-[profile-import_1.4s_ease-in-out_infinite] rounded-full bg-gradient-to-r from-sky-400 to-violet-400" />
                </div>
              )}
            </div>
          )}
          <div className="mt-auto flex flex-wrap gap-1 pt-4">
            <Button
              disabled={importing || busy !== null}
              onClick={() =>
                void run("import", async () => {
                  await importScrobbleHistory(service.service);
                  toast.success(`Importing your ${service.name} history`, {
                    description:
                      "Matched listens join this profile's history and shape Radio.",
                  });
                })
              }
              size="sm"
              variant="ghost"
            >
              <DownloadCloud /> Import history
            </Button>
            {(service.queue.pending > 0 || service.queue.failed > 0) && (
              <Button
                disabled={busy !== null}
                onClick={() =>
                  void run("retry", async () => {
                    await retryScrobbles();
                  })
                }
                size="sm"
                variant="ghost"
              >
                <RotateCw /> Retry now
              </Button>
            )}
            <Button
              className="ml-auto hover:text-rose-300"
              disabled={busy !== null}
              onClick={() =>
                void run("disconnect", async () => {
                  await disconnectScrobbling(service.service);
                  toast.success(`${service.name} disconnected`);
                })
              }
              size="sm"
              variant="ghost"
            >
              <Unplug /> Disconnect
            </Button>
          </div>
        </>
      )}
    </article>
  );
}

function LastfmServerSetup({ status }: { status: ScrobblingStatus }) {
  const queryClient = useQueryClient();
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const setup = status.lastfmSetup;
  const valid =
    plausibleLastfmCredential(apiKey) && plausibleLastfmCredential(apiSecret);

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      await setLastfmServerCredentials(apiKey, apiSecret);
      setApiKey("");
      setApiSecret("");
      toast.success("Last.fm API key saved", {
        description: "Everyone on this server can now connect Last.fm.",
      });
      await queryClient.invalidateQueries({ queryKey: scrobblingKey });
    } catch (cause) {
      setError(errorMessage(cause, "Could not save the API key."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="mt-8 rounded-2xl border border-white/6 bg-white/[0.015] p-5">
      <div className="flex items-start gap-3">
        <KeyRound className="mt-0.5 h-4 w-4 text-zinc-400" />
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold text-white">
            Last.fm API account
          </h3>
          <p className="mt-1 text-xs leading-5 text-zinc-500">
            Last.fm asks every application for its own API account. Create one
            at{" "}
            <a
              className="text-zinc-300 underline-offset-2 hover:underline"
              href="https://www.last.fm/api/account/create"
              rel="noreferrer"
              target="_blank"
            >
              last.fm/api/account/create
            </a>{" "}
            and paste its key and shared secret. They are stored encrypted and
            used for every profile on this server.
          </p>
          {setup.configured ? (
            <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-zinc-300">
              <span className="flex items-center gap-1.5 rounded-full bg-emerald-400/10 px-2.5 py-1 text-emerald-300">
                <CheckCircle2 className="h-3 w-3" />
                {setup.source === "environment"
                  ? "Set by the server's environment"
                  : setup.source === "sandbox"
                    ? "Sandbox key (recorded replies)"
                    : "Saved"}
              </span>
              <code className="rounded bg-white/6 px-1.5 py-0.5 font-mono text-[11px] text-zinc-400">
                {setup.keyHint}
              </code>
              {setup.source === "settings" && (
                <Button
                  onClick={() =>
                    void clearLastfmServerCredentials().then(() =>
                      queryClient.invalidateQueries({
                        queryKey: scrobblingKey,
                      }),
                    )
                  }
                  size="sm"
                  variant="ghost"
                >
                  Remove
                </Button>
              )}
            </div>
          ) : null}
          {setup.source !== "environment" && (
            <form
              className="mt-3 grid gap-2 sm:grid-cols-[1fr_1fr_auto]"
              onSubmit={save}
            >
              <Input
                aria-label="Last.fm API key"
                autoComplete="off"
                onChange={(event) => setApiKey(event.target.value)}
                placeholder="API key"
                spellCheck={false}
                value={apiKey}
              />
              <Input
                aria-label="Last.fm shared secret"
                autoComplete="off"
                onChange={(event) => setApiSecret(event.target.value)}
                placeholder="Shared secret"
                spellCheck={false}
                type="password"
                value={apiSecret}
              />
              <Button disabled={!valid || busy} type="submit">
                {busy && <Loader2 className="animate-spin" />}
                {setup.configured ? "Replace" : "Save"}
              </Button>
            </form>
          )}
          {error && (
            <p className="mt-2 text-xs text-rose-300" role="alert">
              {error}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

function Activity({ status }: { status: ScrobblingStatus }) {
  if (!status.recent.length) {
    return (
      <p className="mt-3 rounded-2xl border border-dashed border-white/8 px-4 py-6 text-center text-xs text-zinc-500">
        Nothing yet. Tracks appear here after you have listened to half of them,
        or four minutes.
      </p>
    );
  }
  return (
    <ol className="mt-3 divide-y divide-white/6 overflow-hidden rounded-2xl border border-white/6">
      {status.recent.map((entry) => {
        const service = status.services.find(
          (candidate) => candidate.service === entry.service,
        );
        const label = activityLabel(entry, service ? service.connection : null);
        return (
          <li className="flex items-center gap-3 px-4 py-2.5" key={entry.id}>
            <ServiceMark service={entry.service} size={22} />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm text-zinc-100">{entry.track}</p>
              <p className="truncate text-xs text-zinc-500">
                {entry.artist}
                {entry.album ? ` · ${entry.album}` : ""}
              </p>
            </div>
            <span className="hidden shrink-0 text-xs tabular-nums text-zinc-500 sm:block">
              {listenTime(entry.listenedAt)}
            </span>
            <span
              className={cn(
                "flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium",
                toneStyles[label.tone],
              )}
              title={entry.lastError ?? undefined}
            >
              {label.tone === "ok" ? (
                <CheckCircle2 className="h-3 w-3" />
              ) : label.tone === "warn" ? (
                <Clock3 className="h-3 w-3" />
              ) : (
                <XCircle className="h-3 w-3" />
              )}
              {label.text}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

export default function ScrobblingSettings() {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const [connecting, setConnecting] = useState(false);
  const popup = useRef<Window | null>(null);
  const status = useQuery({
    queryKey: scrobblingKey,
    queryFn: getScrobbling,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (connecting) return 2_000;
      if (
        data?.services.some((service) => service.import?.status === "running")
      )
        return 1_500;
      if (data?.services.some((service) => service.queue.pending > 0))
        return 10_000;
      return false;
    },
  });
  const lastfm = status.data?.services.find(
    (service) => service.service === "lastfm",
  );

  // Stop waiting once Last.fm shows up as connected.
  useEffect(() => {
    if (connecting && lastfm?.connection?.status === "ok") {
      setConnecting(false);
      popup.current?.close();
      toast.success(`Last.fm connected as ${lastfm.connection.accountName}`);
    }
  }, [connecting, lastfm?.connection]);

  useEffect(() => {
    const finished = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      if ((event.data as { type?: string })?.type === "parson:lastfm-connected")
        void queryClient.invalidateQueries({ queryKey: scrobblingKey });
    };
    window.addEventListener("message", finished);
    return () => window.removeEventListener("message", finished);
  }, [queryClient]);

  const connectLastfm = async () => {
    // The desktop app opens Last.fm in the system browser; a browser gets a
    // popup, opened before any await so popup blockers allow it.
    const desktop = hasDesktopBridge();
    const opened = desktop
      ? null
      : window.open(
          "about:blank",
          "parson-lastfm",
          "popup,width=520,height=720",
        );
    try {
      const { authorizeUrl } = await authorizeLastfm(window.location.origin);
      if (desktop) {
        window.open(authorizeUrl, "_blank", "noopener,noreferrer");
      } else if (opened) {
        opened.location.href = authorizeUrl;
        popup.current = opened;
      } else {
        window.location.href = authorizeUrl;
      }
      setConnecting(true);
      window.setTimeout(() => setConnecting(false), 15 * 60 * 1000);
    } catch (cause) {
      opened?.close();
      toast.error(errorMessage(cause, "Could not start connecting Last.fm."));
    }
  };

  return (
    <section aria-labelledby="scrobbling-title" className="max-w-3xl">
      <div>
        <h2
          className="text-base font-semibold text-white"
          id="scrobbling-title"
        >
          Scrobbling
        </h2>
        <p className="mt-1 max-w-xl text-sm leading-6 text-zinc-400">
          Scrobble this profile&apos;s listening to Last.fm and ListenBrainz.
          Every profile connects its own accounts, so everyone&apos;s scrobbles
          stay their own.
        </p>
        {status.data?.sandbox && (
          <p className="mt-3 inline-flex items-center gap-2 rounded-full bg-violet-400/10 px-3 py-1 text-xs text-violet-200">
            <FlaskConical className="h-3.5 w-3.5" />
            Sandbox: services answer with recorded replies.
          </p>
        )}
      </div>

      {status.isPending ? (
        <div className="mt-6 grid gap-4 md:grid-cols-2">
          {[0, 1].map((index) => (
            <div
              className="h-64 animate-pulse rounded-2xl bg-white/[0.03]"
              key={index}
            />
          ))}
        </div>
      ) : status.isError || !status.data ? (
        <p className="mt-6 text-sm text-rose-300">
          Scrobbling settings could not be loaded.
        </p>
      ) : (
        <>
          <div className="mt-6 grid gap-4 md:grid-cols-2">
            {status.data.services.map((service) => (
              <ServiceCard
                connecting={connecting && service.service === "lastfm"}
                key={service.service}
                onCancelConnect={() => {
                  setConnecting(false);
                  popup.current?.close();
                }}
                onConnectLastfm={() => void connectLastfm()}
                service={service}
              />
            ))}
          </div>
          <p className="mt-4 text-xs leading-5 text-zinc-500">
            Parson follows Last.fm&apos;s rules: a track longer than 30 seconds
            scrobbles once you have heard half of it or four minutes, stamped
            with when you started listening. While a service or this server is
            offline, listens wait in a queue and are sent later with their
            original times.
          </p>

          <h3 className="mt-8 text-sm font-semibold text-white">
            Recent activity
          </h3>
          <Activity status={status.data} />

          {session?.role === "admin" && (
            <LastfmServerSetup status={status.data} />
          )}
        </>
      )}
    </section>
  );
}
