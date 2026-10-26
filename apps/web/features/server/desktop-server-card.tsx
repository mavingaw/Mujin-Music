"use client";

import { Button } from "@/components/ui/button";
import {
  chooseDesktopServer,
  desktopServerState,
  hasDesktopBridge,
  runDesktopLocalServer,
  type DesktopServerState,
} from "@/lib/desktop/bridge";
import { ArrowLeftRight, Monitor, Server } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

/**
 * Parson Desktop only: shows whether this app runs the server or is a remote
 * for one elsewhere, and switches between the two.
 */
export default function DesktopServerCard() {
  const [state, setState] = useState<DesktopServerState | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!hasDesktopBridge()) return;
    let active = true;
    void desktopServerState().then((next) => {
      if (active) setState(next);
    });
    return () => {
      active = false;
    };
  }, []);
  if (!state?.mode) return null;
  const remote = state.mode === "remote";

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
      className="flex flex-col gap-4 rounded-2xl border border-white/10 bg-white/[0.025] p-5 sm:flex-row sm:items-center"
    >
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
            : "Your library lives here. Phones, tablets and other computers connect to it with the address below."}
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
    </section>
  );
}
