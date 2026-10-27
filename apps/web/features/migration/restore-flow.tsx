"use client";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  listFolderBackups,
  listSetupBackups,
  restartServer,
  restoreBackupFromFolder,
  restorePrivateBackup,
  restoreSetupBackup,
  restoreSetupBackupFromFolder,
} from "@parson/music-sdk";
import { CheckCircle2, Loader2 } from "lucide-react";
import { useState } from "react";
import BackupPicker, { type PickedBackup } from "./backup-picker";
import {
  readServerBootId,
  remoteAccessChoice,
  waitForRestart,
} from "./migration-presentation";

type Phase =
  | { kind: "pick" }
  | { kind: "working"; label: string }
  | { kind: "done" }
  | { kind: "error"; message: string };

/**
 * "Restore a backup": pick a backup from a folder/NAS or upload one, then the
 * server validates it, restarts itself and comes back as the old library.
 *
 * `setup` runs before any account exists (first run of a new server);
 * `admin` runs from Settings → Data on a server that is already set up.
 */
export default function RestoreFlow({
  mode,
  onCancel,
  onRestored,
}: {
  mode: "setup" | "admin";
  onCancel?: () => void;
  onRestored: () => void;
}) {
  const [picked, setPicked] = useState<PickedBackup | null>(null);
  const [adoptIdentity, setAdoptIdentity] = useState(mode === "setup");
  // Off by default: the new server may be on another network.
  const [restoreRemoteAccess, setRestoreRemoteAccess] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: "pick" });
  const libraryName =
    picked?.kind === "folder" ? picked.backup.library_name : null;
  const remoteAccess = remoteAccessChoice(picked);

  async function restore() {
    if (!picked) return;
    const remoteChoice = remoteAccessChoice(picked);
    const options = {
      adoptIdentity,
      restoreRemoteAccess: Boolean(remoteChoice) && restoreRemoteAccess,
    };
    setPhase({ kind: "working", label: "Checking the backup…" });
    try {
      const previousBootId = await readServerBootId().catch(() => null);
      const result =
        mode === "setup"
          ? picked.kind === "folder"
            ? await restoreSetupBackupFromFolder(picked.backup.path, options)
            : await restoreSetupBackup(picked.file, options)
          : picked.kind === "folder"
            ? await restoreBackupFromFolder(picked.backup.path, options)
            : await restorePrivateBackup(picked.file, options);
      if (!result.restarting) await restartServer();
      setPhase({
        kind: "working",
        label: "Restoring your library. Parson is restarting…",
      });
      const back = await waitForRestart({
        previousBootId,
        readBootId: () => readServerBootId(),
      });
      if (!back) {
        setPhase({
          kind: "error",
          message:
            "The backup is staged, but the server has not come back yet. If it runs without a supervisor, start it again to finish the restore.",
        });
        return;
      }
      setPhase({ kind: "done" });
      setTimeout(onRestored, 1_200);
    } catch (cause) {
      setPhase({
        kind: "error",
        message:
          cause instanceof Error
            ? cause.message
            : "The backup could not be restored.",
      });
    }
  }

  if (phase.kind === "working" || phase.kind === "done") {
    return (
      <div
        aria-live="polite"
        className="flex flex-col items-center gap-4 py-10 text-center"
      >
        {phase.kind === "done" ? (
          <CheckCircle2 className="h-9 w-9 text-emerald-400" />
        ) : (
          <Loader2 className="h-9 w-9 animate-spin text-zinc-400" />
        )}
        <p className="text-sm text-zinc-300">
          {phase.kind === "done"
            ? "Restored. Sign in with your account from the old server."
            : phase.label}
        </p>
      </div>
    );
  }

  return (
    <div className="grid gap-5">
      <BackupPicker
        list={mode === "setup" ? listSetupBackups : listFolderBackups}
        onSelect={setPicked}
        selected={picked}
      />
      <label className="flex items-start gap-3 rounded-xl border border-white/10 bg-white/[0.03] p-4 text-sm">
        <Checkbox
          checked={adoptIdentity}
          className="mt-0.5"
          onCheckedChange={(value) => setAdoptIdentity(value === true)}
        />
        <span className="leading-5">
          <span className="font-medium text-zinc-100">
            Take over {libraryName ? `“${libraryName}”` : "the old library"}
          </span>
          <span className="mt-0.5 block text-zinc-500">
            Phones and computers that used the old server find this one as the
            same library, at its new address. Switch the old server off first.
          </span>
        </span>
      </label>
      {remoteAccess ? (
        <label className="flex items-start gap-3 rounded-xl border border-white/10 bg-white/[0.03] p-4 text-sm">
          <Checkbox
            checked={restoreRemoteAccess}
            className="mt-0.5"
            onCheckedChange={(value) => setRestoreRemoteAccess(value === true)}
          />
          <span className="leading-5">
            <span className="font-medium text-zinc-100">
              {remoteAccess.label}
            </span>
            <span className="mt-0.5 block text-zinc-500">
              {remoteAccess.detail}
            </span>
          </span>
        </label>
      ) : null}
      <p className="text-xs leading-5 text-zinc-500">
        Accounts, playlists, favourites, listening history, edits and artwork
        come back. Music files are not in a backup: if they live somewhere new
        on this server, point Parson at them in Settings → Library afterwards.
        The current data here is kept as a rollback copy.
      </p>
      {phase.kind === "error" ? (
        <p className="text-sm text-red-300" role="alert">
          {phase.message}
        </p>
      ) : null}
      <div className="flex flex-wrap justify-end gap-2">
        {onCancel ? (
          <Button onClick={onCancel} variant="outline">
            Cancel
          </Button>
        ) : null}
        <Button disabled={!picked} onClick={() => void restore()}>
          Restore and restart
        </Button>
      </div>
    </div>
  );
}
