"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { LibraryFolderBrowser } from "@/features/settings/library-folder-browser";
import { cn } from "@/lib/cn";
import {
  getBackupSchedule,
  updateBackupSchedule,
  type BackupFrequency,
  type BackupScheduleStatus,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, HardDrive, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { scheduleSummary } from "./schedule-presentation";

const FREQUENCIES: { value: BackupFrequency; label: string }[] = [
  { value: "daily", label: "Daily" },
  { value: "weekly", label: "Weekly" },
  { value: "off", label: "Off" },
];

/** Settings → Data → Scheduled backups, with a copy to a folder or NAS. */
export default function BackupSchedule() {
  const queryClient = useQueryClient();
  const schedule = useQuery({
    queryKey: ["backup-schedule"],
    queryFn: getBackupSchedule,
  });
  const [destination, setDestination] = useState("");
  const [keep, setKeep] = useState("10");
  const [browse, setBrowse] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const data = schedule.data;

  useEffect(() => {
    if (!data) return;
    setDestination(data.destination ?? "");
    setKeep(String(data.destination_keep));
  }, [data]);

  async function save(
    change: Partial<{
      frequency: BackupFrequency;
      destination: string | null;
    }>,
  ) {
    if (!data || busy) return;
    setBusy(true);
    setError(null);
    try {
      const next: BackupScheduleStatus = await updateBackupSchedule({
        frequency: change.frequency ?? data.frequency,
        destination:
          change.destination !== undefined
            ? change.destination
            : destination.trim() || null,
        destination_keep: Math.max(1, Number(keep) || 10),
      });
      queryClient.setQueryData(["backup-schedule"], next);
      void queryClient.invalidateQueries({ queryKey: ["data-status"] });
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not save the schedule.",
      );
    } finally {
      setBusy(false);
    }
  }

  const summary = data ? scheduleSummary(data) : null;

  return (
    <section className="border-t border-white/8 pt-8">
      <h2 className="text-base font-semibold text-white">Scheduled backups</h2>
      <p className="mt-1 max-w-xl text-sm text-zinc-500">
        Parson backs up accounts, playlists, history and edits on its own. Add a
        folder on another drive or a NAS share so a copy survives if this
        machine doesn’t, and so a new server can restore it.
      </p>

      <div
        aria-label="How often to back up"
        className="mt-5 inline-grid grid-cols-3 rounded-full border border-white/10 bg-black/40 p-1 text-xs font-semibold"
        role="radiogroup"
      >
        {FREQUENCIES.map(({ value, label }) => (
          <button
            aria-checked={data?.frequency === value}
            className={cn(
              "h-8 rounded-full px-4 transition-colors disabled:opacity-50",
              data?.frequency === value
                ? "bg-white text-black"
                : "text-zinc-400 hover:text-white",
            )}
            disabled={!data || busy}
            key={value}
            onClick={() => void save({ frequency: value })}
            role="radio"
            type="button"
          >
            {label}
          </button>
        ))}
      </div>

      <form
        className="mt-5 grid gap-3 rounded-xl border border-white/10 p-4"
        onSubmit={(event) => {
          event.preventDefault();
          void save({});
        }}
      >
        <label
          className="flex items-center gap-2 text-sm font-medium text-zinc-200"
          htmlFor="backup-destination"
        >
          <HardDrive className="h-4 w-4 text-zinc-500" />
          Also copy each backup to
        </label>
        <div className="flex flex-wrap gap-2">
          <Input
            autoComplete="off"
            className="min-w-0 flex-1 font-mono text-[13px]"
            id="backup-destination"
            onChange={(event) => setDestination(event.target.value)}
            placeholder="/mnt/nas/parson-backups"
            spellCheck={false}
            value={destination}
          />
          <Button
            aria-label="Browse folders on the server"
            onClick={() => setBrowse(true)}
            size="icon"
            type="button"
            variant="secondary"
          >
            <FolderOpen />
          </Button>
          <label className="flex items-center gap-2 text-sm text-zinc-500">
            keep
            <Input
              aria-label="Backups to keep in that folder"
              className="h-10 w-16 text-center"
              inputMode="numeric"
              onChange={(event) =>
                setKeep(event.target.value.replace(/\D/g, "").slice(0, 3))
              }
              value={keep}
            />
          </label>
          <Button disabled={!data || busy} type="submit">
            {busy ? <Loader2 className="animate-spin" /> : null}
            Save
          </Button>
          {data?.destination ? (
            <Button
              disabled={busy}
              onClick={() => {
                setDestination("");
                void save({ destination: null });
              }}
              type="button"
              variant="ghost"
            >
              Stop copying
            </Button>
          ) : null}
        </div>
        {summary ? (
          <p
            className={cn(
              "flex items-start gap-2 text-xs leading-5",
              summary.tone === "warn" ? "text-amber-200/90" : "text-zinc-500",
            )}
          >
            <span
              aria-hidden="true"
              className={cn(
                "mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full",
                summary.tone === "good" && "bg-emerald-400",
                summary.tone === "warn" && "bg-amber-400",
                summary.tone === "off" && "bg-zinc-600",
              )}
            />
            {summary.text}
          </p>
        ) : null}
        {error ? (
          <p className="text-sm text-red-300" role="alert">
            {error}
          </p>
        ) : null}
      </form>

      <Dialog open={browse} onOpenChange={setBrowse}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Choose a backup folder</DialogTitle>
            <DialogDescription>
              Folders this server can see. A NAS share has to be mounted on the
              server to show up here.
            </DialogDescription>
          </DialogHeader>
          <LibraryFolderBrowser
            busy={false}
            initialDirectory={destination.trim() || "/"}
            onCancel={() => setBrowse(false)}
            onSelect={(path) => {
              setDestination(path);
              setBrowse(false);
            }}
          />
        </DialogContent>
      </Dialog>
    </section>
  );
}
