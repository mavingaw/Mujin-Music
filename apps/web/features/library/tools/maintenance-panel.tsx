"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import {
  cancelToolJob,
  forgetMissingFiles,
  getMissingFiles,
  reanalyzeLibraryItems,
  reidentifyLibraryItems,
  rescanLibraryItems,
  type ToolJob,
  type ToolJobKind,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AudioWaveform,
  CheckCircle2,
  FolderX,
  HardDrive,
  Loader2,
  RefreshCcw,
  ScanSearch,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useState, type ReactNode } from "react";
import { toast } from "sonner";
import { getApiErrorMessage } from "./tool-errors";
import { EmptyState, JobProgress, ToolCard } from "./tool-primitives";
import {
  forgetButtonLabel,
  formatBytes,
  formatCount,
  formatServerTime,
  runningJob,
} from "./tools-model";
import { useStartToolJob } from "./use-tool-jobs";

const missingKey = ["library-tools", "missing"] as const;

export function MaintenancePanel({
  jobs,
  lastScanAt,
}: {
  jobs: ToolJob[];
  lastScanAt: string | null;
}) {
  const startJob = useStartToolJob();
  const [confirmRescan, setConfirmRescan] = useState(false);
  const finished = jobs.filter((job) => job.state !== "running").slice(0, 6);

  return (
    <div className="space-y-6">
      <div className="grid gap-4 lg:grid-cols-3">
        <Operation
          description={`Reads every file again: tags, stream headers and folder artwork.${lastScanAt ? ` Last scan ${formatServerTime(lastScanAt)}.` : ""}`}
          icon={<RefreshCcw />}
          job={runningJob(jobs, "rescan")}
          kind="rescan"
          title="Rescan library"
        >
          <Button onClick={() => setConfirmRescan(true)} variant="secondary">
            <RefreshCcw />
            Rescan everything
          </Button>
        </Operation>
        <Operation
          description="Looks every album up on MusicBrainz again, as on first import. Albums you identified by hand keep their release."
          icon={<ScanSearch />}
          job={runningJob(jobs, "reidentify")}
          kind="reidentify"
          title="Re-identify albums"
        >
          <Button
            onClick={() =>
              void startJob(
                () => reidentifyLibraryItems({ scope: "library" }),
                "Could not start identification.",
              )
            }
            variant="secondary"
          >
            <ScanSearch />
            Re-identify all
          </Button>
        </Operation>
        <Operation
          description="Measures loudness (EBU R128) for leveling, draws waveforms and fingerprints the audio to confirm duplicates."
          icon={<AudioWaveform />}
          job={runningJob(jobs, "reanalyze")}
          kind="reanalyze"
          title="Analyze audio"
        >
          <Button
            onClick={() =>
              void startJob(
                () => reanalyzeLibraryItems({ scope: "library" }),
                "Could not start the analysis.",
              )
            }
            variant="secondary"
          >
            <AudioWaveform />
            New tracks
          </Button>
          <Button
            onClick={() =>
              void startJob(
                () => reanalyzeLibraryItems({ scope: "library" }, true),
                "Could not start the analysis.",
              )
            }
            variant="ghost"
          >
            Everything again
          </Button>
        </Operation>
      </div>

      <MissingFilesSection />

      {finished.length > 0 && (
        <ToolCard title="Recent jobs">
          <div className="grid gap-2 md:grid-cols-2">
            {finished.map((job) => (
              <JobProgress compact job={job} key={job.id} />
            ))}
          </div>
        </ToolCard>
      )}

      <Dialog onOpenChange={setConfirmRescan} open={confirmRescan}>
        <DialogContent className="[&>*]:min-w-0">
          <DialogHeader>
            <DialogTitle>Rescan the whole library?</DialogTitle>
            <DialogDescription>
              Every file is read again from disk. Large libraries take a while;
              playback keeps working meanwhile. Files are only read, never
              changed.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button onClick={() => setConfirmRescan(false)} variant="ghost">
              Cancel
            </Button>
            <Button
              onClick={() => {
                setConfirmRescan(false);
                void startJob(
                  () => rescanLibraryItems({ scope: "library" }),
                  "Could not start the rescan.",
                );
              }}
            >
              <RefreshCcw />
              Rescan
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function Operation({
  title,
  description,
  icon,
  job,
  children,
}: {
  title: string;
  description: string;
  icon: ReactNode;
  kind: ToolJobKind;
  job: ToolJob | undefined;
  children: ReactNode;
}) {
  return (
    <ToolCard className="flex flex-col">
      <div className="flex items-start gap-3">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[#b26bff]/10 text-[#b26bff] [&_svg]:h-5 [&_svg]:w-5">
          {icon}
        </span>
        <div className="min-w-0">
          <h2 className="text-[15px] font-semibold text-white">{title}</h2>
          <p className="mt-1 text-sm leading-5 text-zinc-500">{description}</p>
        </div>
      </div>
      <div className="mt-auto pt-5">
        {job ? (
          <JobProgress
            compact
            job={job}
            onCancel={() => void cancelToolJob(job.id)}
          />
        ) : (
          <div className="flex flex-wrap gap-2">{children}</div>
        )}
      </div>
    </ToolCard>
  );
}

function MissingFilesSection() {
  const queryClient = useQueryClient();
  const missing = useQuery({ queryKey: missingKey, queryFn: getMissingFiles });
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const data = missing.data;
  const offline = data?.roots.filter((root) => !root.reachable) ?? [];
  const forgettable = data?.forgettable ?? 0;
  const forgettableBytes =
    data?.files
      .filter((file) => file.root_reachable && !file.present_again)
      .reduce((sum, file) => sum + file.size_bytes, 0) ?? 0;

  const forget = async () => {
    setBusy(true);
    try {
      const forgotten = await forgetMissingFiles(forgettable);
      toast.success(
        `Forgot ${formatCount(forgotten)} missing file${forgotten === 1 ? "" : "s"}`,
      );
      setConfirm(false);
      await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
      await invalidateCatalogRevisionQueries(queryClient);
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not forget the missing files."));
      await missing.refetch();
    } finally {
      setBusy(false);
    }
  };

  return (
    <ToolCard
      action={
        forgettable > 0 ? (
          <Button onClick={() => setConfirm(true)} variant="secondary">
            <Trash2 />
            {forgetButtonLabel(forgettable)}
          </Button>
        ) : undefined
      }
      description="Files the library remembers but can no longer find. Forgetting removes them from Parson's records only; nothing on disk is touched. Files on a drive or share that is offline are always kept."
      title={
        <span className="flex items-baseline gap-2">
          Missing files
          {data && (
            <span className="text-sm font-normal tabular-nums text-zinc-500">
              {formatCount(data.files.length)}
            </span>
          )}
        </span>
      }
    >
      {offline.length > 0 && (
        <div className="mb-4 space-y-2">
          {offline.map((root) => (
            <div
              className="flex items-center gap-3 rounded-lg border border-[#f5c542]/25 bg-[#f5c542]/[0.06] px-3 py-2.5 text-sm text-[#f5d27a]"
              key={root.path}
            >
              <HardDrive className="h-4 w-4 shrink-0" />
              <span className="min-w-0 flex-1">
                <span className="font-medium">Offline: </span>
                <span className="font-mono text-xs">{root.path}</span>
                <span className="block text-xs text-[#f5d27a]/70">
                  Its files are kept until the drive is back.
                </span>
              </span>
            </div>
          ))}
        </div>
      )}
      {missing.isPending ? (
        <div className="flex h-24 items-center justify-center text-zinc-500">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      ) : !data?.files.length ? (
        <EmptyState icon={<CheckCircle2 />} title="No missing files">
          Every file the library knows is where it should be.
        </EmptyState>
      ) : (
        <ul className="max-h-96 divide-y divide-white/[0.05] overflow-y-auto rounded-lg border border-white/[0.06]">
          {data.files.slice(0, 500).map((file) => (
            <li className="flex items-center gap-3 px-3 py-2" key={file.path}>
              <FolderX
                className={`h-4 w-4 shrink-0 ${file.root_reachable ? "text-zinc-500" : "text-[#f5c542]"}`}
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm text-zinc-200">
                  {file.title}
                  {(file.artist || file.album) && (
                    <span className="text-zinc-500">
                      {" "}
                      · {[file.artist, file.album].filter(Boolean).join(" · ")}
                    </span>
                  )}
                </p>
                <p className="truncate font-mono text-[11px] text-zinc-600">
                  {file.path}
                </p>
              </div>
              <span className="shrink-0 text-[11px] text-zinc-500">
                {file.present_again
                  ? "Back on disk"
                  : !file.root_reachable
                    ? "Drive offline"
                    : formatBytes(file.size_bytes)}
              </span>
            </li>
          ))}
        </ul>
      )}

      <Dialog onOpenChange={setConfirm} open={confirm}>
        <DialogContent className="[&>*]:min-w-0">
          <DialogHeader>
            <DialogTitle>{forgetButtonLabel(forgettable)}?</DialogTitle>
            <DialogDescription>
              Parson stops remembering {formatCount(forgettable)} file
              {forgettable === 1 ? "" : "s"} ({formatBytes(forgettableBytes)})
              it can no longer find. Nothing on disk is deleted. If a file comes
              back, the next scan adds it again; its likes and playlist entries
              return with it.
            </DialogDescription>
          </DialogHeader>
          <div className="flex items-start gap-3 rounded-lg border border-white/[0.06] bg-black/30 p-3 text-xs leading-5 text-zinc-400">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-[#3ddc97]" />
            Files on offline drives and files that reappeared are kept. If the
            list changed since you opened it, nothing is forgotten.
          </div>
          <DialogFooter>
            <Button onClick={() => setConfirm(false)} variant="ghost">
              Cancel
            </Button>
            <Button
              disabled={busy}
              onClick={() => void forget()}
              variant="destructive"
            >
              {busy ? <Loader2 className="animate-spin" /> : <Trash2 />}
              {forgetButtonLabel(forgettable)}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </ToolCard>
  );
}
