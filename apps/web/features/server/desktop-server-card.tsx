"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { createPrivateBackup, type BackupSummary } from "@parson/music-sdk";
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
import { remoteRoleCopy, serviceSummary } from "./desktop-service";
import {
  ArchiveRestore,
  ArrowLeftRight,
  CheckCircle2,
  Loader2,
  Monitor,
  Server,
} from "lucide-react";
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
  const [moveOpen, setMoveOpen] = useState(false);
  const [moveBackup, setMoveBackup] = useState<BackupSummary | null>(null);
  const [backingUp, setBackingUp] = useState(false);
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
  const remoteCopy = remoteRoleCopy(state);

  async function backUpForMove() {
    setBackingUp(true);
    try {
      setMoveBackup(await createPrivateBackup());
    } catch (error) {
      toast(
        error instanceof Error ? error.message : "The backup was not created.",
      );
    } finally {
      setBackingUp(false);
    }
  }

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
              ? remoteCopy
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
              {state.localRunning
                ? "Open this computer’s server"
                : "Run server here"}
            </Button>
          ) : null}
          <Button
            className="rounded-full"
            disabled={busy}
            onClick={() =>
              remote ? void run(chooseDesktopServer) : setMoveOpen(true)
            }
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
      <Dialog
        open={moveOpen}
        onOpenChange={(open) => {
          if (!backingUp) setMoveOpen(open);
        }}
      >
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Use another server</DialogTitle>
            <DialogDescription>
              Moving this library to a NAS or a new computer? Back it up first,
              then restore it there while setting it up. Accounts, playlists,
              favourites, history and edits come along.
            </DialogDescription>
          </DialogHeader>
          <ol className="grid gap-3 text-sm">
            <li className="flex items-start gap-3 rounded-xl border border-white/10 bg-white/[0.03] p-4">
              <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-white text-xs font-semibold text-black">
                1
              </span>
              <div className="min-w-0 flex-1">
                <p className="font-medium text-zinc-100">
                  Back up this library
                </p>
                {moveBackup ? (
                  <p className="mt-1 flex items-start gap-1.5 text-xs leading-5 text-emerald-300/90">
                    <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    {moveBackup.copied_to
                      ? `Saved and copied to ${moveBackup.copied_to}.`
                      : "Saved in this server’s backup folder. Copy it to the new server, or set a NAS folder under Settings → Data so it lands there on its own."}
                    {moveBackup.copy_error
                      ? ` The copy to your backup folder failed: ${moveBackup.copy_error}`
                      : ""}
                  </p>
                ) : (
                  <Button
                    className="mt-2"
                    disabled={backingUp}
                    onClick={() => void backUpForMove()}
                    size="sm"
                  >
                    {backingUp ? <Loader2 className="animate-spin" /> : null}
                    {backingUp ? "Backing up…" : "Back up now"}
                  </Button>
                )}
              </div>
            </li>
            <li className="flex items-start gap-3 rounded-xl border border-white/10 bg-white/[0.03] p-4">
              <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-white/15 text-xs font-semibold text-white">
                2
              </span>
              <p className="min-w-0 flex-1 leading-5 text-zinc-400">
                <span className="font-medium text-zinc-100">
                  On the new server
                </span>
                , choose{" "}
                <ArchiveRestore className="inline h-3.5 w-3.5 align-[-2px]" />{" "}
                “Moving from another Parson server?” when it asks for the first
                account, and pick this backup.
              </p>
            </li>
          </ol>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              disabled={backingUp}
              onClick={() => {
                setMoveOpen(false);
                void run(chooseDesktopServer);
              }}
              variant={moveBackup ? "default" : "outline"}
            >
              {moveBackup ? "Continue to servers" : "Switch without a backup"}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </section>
  );
}
