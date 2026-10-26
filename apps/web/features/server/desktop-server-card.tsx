"use client";

import { Button } from "@/components/ui/button";
import {
  chooseDesktopServer,
  desktopServiceState,
  desktopServerState,
  hasDesktopBridge,
  runDesktopLocalServer,
  updateDesktopService,
  type DesktopServerState,
  type DesktopServiceState,
} from "@/lib/desktop/bridge";
import { serviceSummary } from "./desktop-service";
import { ArrowLeftRight, Monitor, Server } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

/**
 * Parson Desktop only: shows whether this app runs the server or is a remote
 * for one elsewhere, and switches between the two.
 */
export default function DesktopServerCard() {
  const [state, setState] = useState<DesktopServerState | null>(null);
  const [service, setService] = useState<DesktopServiceState | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!hasDesktopBridge()) return;
    let active = true;
    void desktopServerState().then((next) => {
      if (active) setState(next);
      if (next?.mode === "local") {
        void desktopServiceState().then((value) => {
          if (active) setService(value);
        });
      }
    });
    return () => {
      active = false;
    };
  }, []);
  if (!state?.mode) return null;
  const remote = state.mode === "remote";
  const summary = remote ? null : serviceSummary(service);

  async function changeService(
    change: Parameters<typeof updateDesktopService>[0],
  ) {
    setBusy(true);
    try {
      setService(await updateDesktopService(change));
    } catch (error) {
      toast(
        error instanceof Error ? error.message : "Could not change the server.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function run(action: () => Promise<boolean>) {
    setBusy(true);
    try {
      await action();
    } catch (error) {
      toast(error instanceof Error ? error.message : "Could not switch.");
      setBusy(false);
    }
  }

  return (
    <section
      aria-labelledby="desktop-server-heading"
      className="rounded-2xl border border-white/10 bg-white/[0.025] p-5"
    >
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-white/[0.06] text-zinc-200">
          {remote ? (
            <Server className="h-5 w-5" />
          ) : (
            <Monitor className="h-5 w-5" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <h2
            className="text-sm font-semibold text-white"
            id="desktop-server-heading"
          >
            {remote
              ? `This app is a remote for ${state.name}`
              : "This computer runs the Parson server"}
          </h2>
          <p className="mt-0.5 text-sm leading-5 text-zinc-500">
            {remote
              ? "No server runs here. Your library, queue and zones live on that server and stay in sync with every other app."
              : "It runs in the background, apart from this window. Close or quit Parson and your phones, browsers and other computers keep their library and zones."}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {remote && state.localAvailable ? (
            <Button
              disabled={busy}
              onClick={() => void run(runDesktopLocalServer)}
              size="sm"
              variant="ghost"
            >
              Run server here
            </Button>
          ) : null}
          <Button
            className="rounded-full"
            disabled={busy}
            onClick={() => void run(chooseDesktopServer)}
            size="sm"
          >
            <ArrowLeftRight />
            {remote ? "Switch server" : "Use another server"}
          </Button>
        </div>
      </div>
      {summary ? (
        <div className="mt-4 flex flex-col gap-3 border-t border-white/[0.06] pt-4 sm:flex-row sm:items-center">
          <div className="flex min-w-0 flex-1 items-start gap-3">
            <span
              aria-hidden="true"
              className={
                summary.running
                  ? "mt-1.5 h-2 w-2 shrink-0 rounded-full bg-emerald-400 shadow-[0_0_0_3px_rgba(52,211,153,0.15)]"
                  : "mt-1.5 h-2 w-2 shrink-0 rounded-full bg-zinc-600"
              }
            />
            <div className="min-w-0">
              <p className="text-sm font-medium text-zinc-200">
                {summary.status}
              </p>
              <p className="text-xs leading-5 text-zinc-500">
                {summary.detail}
              </p>
            </div>
          </div>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            <button
              aria-checked={Boolean(service?.autostart)}
              className="flex items-center gap-2 rounded-full px-2 py-1 text-sm text-zinc-300 transition hover:bg-white/[0.06] disabled:opacity-50"
              disabled={busy}
              onClick={() =>
                void changeService({ autostart: !service?.autostart })
              }
              role="switch"
              type="button"
            >
              <span
                aria-hidden="true"
                className={`relative h-5 w-9 rounded-full transition ${service?.autostart ? "bg-white" : "bg-white/15"}`}
              >
                <span
                  className={`absolute top-0.5 h-4 w-4 rounded-full transition-all ${service?.autostart ? "left-[18px] bg-black" : "left-0.5 bg-zinc-300"}`}
                />
              </span>
              {summary.autostartLabel}
            </button>
            {summary.running ? (
              <>
                <Button
                  disabled={busy}
                  onClick={() => void changeService({ action: "restart" })}
                  size="sm"
                  variant="ghost"
                >
                  Restart
                </Button>
                <Button
                  disabled={busy}
                  onClick={() => void changeService({ action: "stop" })}
                  size="sm"
                  variant="ghost"
                >
                  Stop server
                </Button>
              </>
            ) : (
              <Button
                disabled={busy}
                onClick={() => void changeService({ action: "start" })}
                size="sm"
              >
                Start server
              </Button>
            )}
          </div>
        </div>
      ) : null}
    </section>
  );
}
