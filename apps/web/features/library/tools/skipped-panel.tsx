"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/cn";
import {
  cancelToolJob,
  checkSkippedFiles,
  createImportJob,
  getSkippedFiles,
  getSkippedPlaylist,
  ignoreSkippedFiles,
  retrySkippedFiles,
  type SkippedFile,
  type SkipReason,
  type ToolJob,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  ChevronDown,
  ClipboardCopy,
  Disc3,
  Eye,
  EyeOff,
  FileCheck2,
  FolderSearch,
  ListMusic,
  Loader2,
  RotateCw,
  Search,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { getApiErrorMessage } from "./tool-errors";
import { EmptyState, JobProgress, ToolCard } from "./tool-primitives";
import {
  formatBytes,
  formatCount,
  formatServerTime,
  runningJob,
  skippedSections,
  skipReasonHelp,
} from "./tools-model";
import { useStartToolJob } from "./use-tool-jobs";

const skippedKey = ["library-tools", "skipped"] as const;

export function SkippedPanel({ jobs }: { jobs: ToolJob[] }) {
  const queryClient = useQueryClient();
  const startJob = useStartToolJob();
  const skipped = useQuery({ queryKey: skippedKey, queryFn: getSkippedFiles });
  const [reason, setReason] = useState<SkipReason | null>(null);
  const [query, setQuery] = useState("");
  const [showIgnored, setShowIgnored] = useState(false);
  const checking = runningJob(jobs, "skipped_check");
  const rescanning = runningJob(jobs, "rescan");
  const sections = useMemo(
    () =>
      skippedSections(skipped.data?.items ?? [], {
        reason,
        query,
        showIgnored,
      }),
    [skipped.data, reason, query, showIgnored],
  );
  const ignoredCount =
    skipped.data?.items.filter((item) => item.ignored).length ?? 0;
  const total =
    skipped.data?.reasons.reduce((sum, item) => sum + item.count, 0) ?? 0;

  const setIgnored = async (paths: string[], ignored: boolean) => {
    try {
      await ignoreSkippedFiles(paths, ignored);
      await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
      toast.success(
        ignored
          ? `Ignoring ${paths.length === 1 ? "1 file" : `${paths.length} files`}`
          : "Back in the list",
      );
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not update the list."));
    }
  };

  const retry = (paths: string[]) =>
    void startJob(
      () => retrySkippedFiles(paths),
      "Could not read the files again.",
    );

  return (
    <div className="space-y-6">
      <ToolCard
        action={
          <Button
            disabled={Boolean(checking)}
            onClick={() =>
              void startJob(checkSkippedFiles, "Could not check the folders.")
            }
          >
            {checking ? <Loader2 className="animate-spin" /> : <FolderSearch />}
            {skipped.data?.checked_at ? "Check again" : "Check folders"}
          </Button>
        }
        description={
          skipped.data?.checked_at
            ? `Every music folder was checked ${formatServerTime(skipped.data.checked_at)}. Each file shows exactly why it isn't in your library and how to fix it.`
            : "Walks every music folder the way a scan does and lists each file that didn't make it into your library, with the exact reason and a fix."
        }
        title={
          <span className="flex items-baseline gap-2">
            Skipped files
            {total > 0 && (
              <span className="text-sm font-normal tabular-nums text-zinc-500">
                {formatCount(total)}
              </span>
            )}
          </span>
        }
      >
        {checking && (
          <JobProgress
            compact
            job={checking}
            onCancel={() => void cancelToolJob(checking.id)}
          />
        )}
        {rescanning && !checking && (
          <JobProgress
            compact
            job={rescanning}
            onCancel={() => void cancelToolJob(rescanning.id)}
          />
        )}
        {(skipped.data?.reasons.length ?? 0) > 0 && (
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <ReasonChip
              active={reason === null}
              count={total}
              label="All"
              onClick={() => setReason(null)}
            />
            {skipped.data?.reasons.map((item) => (
              <ReasonChip
                active={reason === item.reason}
                count={item.count}
                key={item.reason}
                label={item.label}
                onClick={() =>
                  setReason(reason === item.reason ? null : item.reason)
                }
              />
            ))}
          </div>
        )}
        {(skipped.data?.items.length ?? 0) > 0 && (
          <div className="mt-4 flex flex-wrap items-center gap-3">
            <div className="relative min-w-56 flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-zinc-500" />
              <Input
                aria-label="Filter skipped files"
                className="h-9 pl-9"
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Filter by path or reason"
                value={query}
              />
            </div>
            {ignoredCount > 0 && (
              <Button
                onClick={() => setShowIgnored(!showIgnored)}
                size="sm"
                variant="ghost"
              >
                {showIgnored ? <EyeOff /> : <Eye />}
                {showIgnored ? "Hide" : "Show"} {formatCount(ignoredCount)}{" "}
                ignored
              </Button>
            )}
          </div>
        )}
      </ToolCard>

      {skipped.isPending ? (
        <div className="flex h-40 items-center justify-center text-zinc-500">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      ) : sections.length === 0 ? (
        <EmptyState
          icon={<FileCheck2 />}
          title={
            skipped.data?.checked_at
              ? "Everything in your folders is in your library"
              : "Not checked yet"
          }
        >
          {skipped.data?.checked_at
            ? "No file was skipped. Ignored files stay hidden until they change."
            : "Check your folders to find files that didn't import."}
        </EmptyState>
      ) : (
        sections.map((section) => (
          <section
            className="overflow-hidden rounded-xl border border-white/[0.07]"
            key={section.reason}
          >
            <header className="flex flex-wrap items-center justify-between gap-3 border-b border-white/[0.06] bg-white/[0.025] px-4 py-3">
              <div className="min-w-0">
                <h3 className="text-sm font-semibold text-white">
                  {section.label}
                  <span className="ml-2 font-normal tabular-nums text-zinc-500">
                    {formatCount(section.items.length)}
                  </span>
                </h3>
                <p className="mt-0.5 text-xs text-zinc-500">
                  {skipReasonHelp[section.reason]}
                </p>
              </div>
              <div className="flex gap-1">
                {section.items.some((item) => item.fix.action === "rescan") && (
                  <Button
                    disabled={Boolean(rescanning)}
                    onClick={() =>
                      retry(section.items.map((item) => item.path))
                    }
                    size="sm"
                    variant="ghost"
                  >
                    <RotateCw />
                    Read all again
                  </Button>
                )}
                <Button
                  onClick={() =>
                    void setIgnored(
                      section.items
                        .filter((item) => !item.ignored)
                        .map((item) => item.path),
                      true,
                    )
                  }
                  size="sm"
                  variant="ghost"
                >
                  <EyeOff />
                  Ignore all
                </Button>
              </div>
            </header>
            <ul className="divide-y divide-white/[0.05]">
              {section.items.slice(0, 300).map((item) => (
                <SkippedRow
                  busy={Boolean(rescanning)}
                  item={item}
                  key={item.path}
                  onIgnore={(ignored) => void setIgnored([item.path], ignored)}
                  onRetry={() => retry([item.path])}
                />
              ))}
            </ul>
            {section.items.length > 300 && (
              <p className="border-t border-white/[0.05] px-4 py-3 text-xs text-zinc-500">
                {formatCount(section.items.length - 300)} more. Filter to narrow
                the list.
              </p>
            )}
          </section>
        ))
      )}
    </div>
  );
}

function ReasonChip({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      aria-pressed={active}
      className={cn(
        "inline-flex h-8 items-center gap-2 rounded-full border px-3 text-xs font-medium transition-colors",
        active
          ? "border-white/20 bg-white text-black"
          : "border-white/[0.08] bg-white/[0.03] text-zinc-300 hover:bg-white/[0.07]",
      )}
      onClick={onClick}
      type="button"
    >
      {label}
      <span
        className={cn(
          "tabular-nums",
          active ? "text-black/60" : "text-zinc-500",
        )}
      >
        {formatCount(count)}
      </span>
    </button>
  );
}

function SkippedRow({
  item,
  busy,
  onIgnore,
  onRetry,
}: {
  item: SkippedFile;
  busy: boolean;
  onIgnore: (ignored: boolean) => void;
  onRetry: () => void;
}) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [importing, setImporting] = useState(false);
  const [copied, setCopied] = useState(false);

  const importPlaylist = async () => {
    setImporting(true);
    try {
      const file = await getSkippedPlaylist(item.path);
      const job = await createImportJob(file);
      toast.success(`Importing ${file.file_name}`);
      router.push(`/imports?job=${encodeURIComponent(job.id)}`);
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not import this playlist."));
    } finally {
      setImporting(false);
    }
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast("Copy is not available here.");
    }
  };

  return (
    <li className={cn("px-4 py-3", item.ignored && "opacity-55")}>
      <div className="flex items-start gap-3">
        <button
          aria-expanded={open}
          aria-label={`How to fix ${item.file_name}`}
          className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-zinc-500 transition-colors hover:bg-white/8 hover:text-white"
          onClick={() => setOpen(!open)}
          type="button"
        >
          <ChevronDown
            className={cn("h-4 w-4 transition-transform", open && "rotate-180")}
          />
        </button>
        <div className="min-w-0 flex-1">
          <p
            className="truncate text-sm font-medium text-zinc-100"
            title={item.path}
          >
            {item.file_name}
          </p>
          <p
            className="truncate font-mono text-[0.6875rem] text-zinc-500"
            title={item.folder}
          >
            {item.folder}
          </p>
          {item.detail && (
            <p className="mt-1 text-xs leading-5 text-zinc-400">
              {item.detail}
            </p>
          )}
        </div>
        <span className="hidden shrink-0 pt-0.5 text-xs tabular-nums text-zinc-600 sm:block">
          {item.size_bytes > 0 ? formatBytes(item.size_bytes) : ""}
        </span>
        <div className="flex shrink-0 items-center gap-1">
          {item.fix.action === "rescan" && (
            <Button
              disabled={busy}
              onClick={onRetry}
              size="sm"
              variant="secondary"
            >
              <RotateCw />
              <span className="hidden md:inline">Read again</span>
            </Button>
          )}
          {item.fix.action === "import_playlist" && (
            <Button
              disabled={importing}
              onClick={() => void importPlaylist()}
              size="sm"
              variant="secondary"
            >
              {importing ? <Loader2 className="animate-spin" /> : <ListMusic />}
              <span className="hidden md:inline">Import playlist</span>
            </Button>
          )}
          {item.fix.action === "edit" && item.album_id && (
            <Button asChild size="sm" variant="secondary">
              <Link href={`/album?id=${encodeURIComponent(item.album_id)}`}>
                <Disc3 />
                <span className="hidden md:inline">Open album</span>
              </Link>
            </Button>
          )}
          <Button
            aria-label={item.ignored ? "Stop ignoring" : "Ignore"}
            onClick={() => onIgnore(!item.ignored)}
            size="sm"
            title={item.ignored ? "Stop ignoring" : "Ignore this file"}
            variant="ghost"
          >
            {item.ignored ? <Eye /> : <EyeOff />}
          </Button>
        </div>
      </div>
      {open && (
        <div className="ml-9 mt-3 rounded-lg border border-white/[0.06] bg-black/40 p-3">
          <p className="text-xs leading-5 text-zinc-300">{item.fix.summary}</p>
          {item.fix.command && (
            <div className="mt-2 flex items-center gap-2 rounded-md bg-white/[0.04] py-1.5 pl-3 pr-1.5">
              <code className="min-w-0 flex-1 truncate font-mono text-[0.6875rem] text-zinc-300">
                {item.fix.command}
              </code>
              <button
                aria-label="Copy command"
                className="flex h-7 w-7 shrink-0 items-center justify-center rounded text-zinc-500 hover:bg-white/8 hover:text-white"
                onClick={() => void copy(item.fix.command ?? "")}
                type="button"
              >
                {copied ? (
                  <Check className="h-3.5 w-3.5 text-quality-high" />
                ) : (
                  <ClipboardCopy className="h-3.5 w-3.5" />
                )}
              </button>
            </div>
          )}
          <p className="mt-2 font-mono text-[0.6875rem] text-zinc-600">
            {item.path}
          </p>
        </div>
      )}
    </li>
  );
}
