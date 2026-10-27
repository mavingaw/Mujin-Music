"use client";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import { cn } from "@/lib/cn";
import {
  cancelEnhanceQueue,
  cancelToolJob,
  cancelUpgradeRequest,
  checkForFakeLossless,
  deleteUpgradeRequest,
  enhanceUnfoundWithAi,
  enhanceWithAi,
  fetchQueryPreview,
  getEnhanceQueue,
  getUpgradeCandidates,
  isUpgradeOpen,
  requestLosslessVersions,
  retryUpgradeRequest,
  searchFetchForAlbum,
  upgradeProgressDetail,
  upgradeStatusLabel,
  type UpgradeCandidate,
  type UpgradeReason,
  type UpgradeRequest,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AudioWaveform,
  Ban,
  Download,
  Loader2,
  RotateCcw,
  Search,
  Settings2,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import Link from "next/link";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
  candidateFilterLabels,
  canRequest,
  matchesFilter,
  reasonColors,
  refreshInterval,
  requestColor,
  requestableSelection,
  type CandidateFilter,
} from "./better-versions-model";
import { getApiErrorMessage } from "./tool-errors";
import {
  EmptyState,
  JobProgress,
  QualityBadge,
  ToolCard,
  ToolCover,
} from "./tool-primitives";
import { formatCount } from "./tools-model";

export const betterVersionsKey = ["library-tools", "better-versions"] as const;

function ReasonBadge({ reason }: { reason: UpgradeReason }) {
  const color = reasonColors[reason.kind];
  return (
    <span
      className="inline-flex h-5 shrink-0 items-center rounded-full border px-2 text-[11px] font-medium"
      data-reason={reason.kind}
      style={{
        borderColor: `color-mix(in srgb, ${color} 45%, transparent)`,
        color,
      }}
      title={reason.detail}
    >
      {reason.label}
    </span>
  );
}

function RequestStatus({ request }: { request: UpgradeRequest }) {
  const open = isUpgradeOpen(request.state);
  const detail = upgradeProgressDetail(request);
  const color = requestColor(request);
  const progress =
    request.state === "downloading" && request.progress
      ? Math.max(2, Math.min(100, request.progress))
      : null;
  return (
    <div className="min-w-0" data-request-state={request.state}>
      <p
        className="flex items-center gap-1.5 truncate text-xs font-medium"
        style={{ color }}
        title={request.message || undefined}
      >
        {open && request.state !== "handed_off" ? (
          <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
        ) : (
          <span
            aria-hidden
            className="h-1.5 w-1.5 shrink-0 rounded-full"
            style={{ backgroundColor: color }}
          />
        )}
        <span className="truncate">{upgradeStatusLabel(request)}</span>
      </p>
      {progress !== null && (
        <div className="mt-1.5 h-1 w-full max-w-56 overflow-hidden rounded-full bg-white/[0.06]">
          <div
            className="h-full rounded-full bg-[linear-gradient(90deg,var(--color-mujin-copper-pressed),var(--color-mujin-copper))] transition-[width] duration-500"
            style={{ width: `${progress}%` }}
          />
        </div>
      )}
      {(detail || (!open && request.message)) && (
        <p className="mt-1 truncate text-[11px] text-zinc-500">
          {open ? detail : request.message}
        </p>
      )}
    </div>
  );
}

function CandidateRow({
  candidate,
  selected,
  onSelect,
  fetchEnabled,
  enhancerEnabled,
  onChanged,
  onRequest,
}: {
  candidate: UpgradeCandidate;
  selected: boolean;
  onSelect: (selected: boolean) => void;
  fetchEnabled: boolean;
  enhancerEnabled: boolean;
  onChanged: () => Promise<void>;
  onRequest: (candidates: UpgradeCandidate[]) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const request = candidate.request;
  const run = async (key: string, work: () => Promise<string | void>) => {
    setBusy(key);
    try {
      const message = await work();
      if (message) toast.success(message);
      await onChanged();
    } catch (error) {
      toast(getApiErrorMessage(error, "That did not work."));
    } finally {
      setBusy(null);
    }
  };
  const search = candidate.search;
  return (
    <li
      className="grid grid-cols-[auto_auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2 border-b border-white/[0.05] px-1 py-3 last:border-0 sm:grid-cols-[auto_auto_minmax(0,1.3fr)_minmax(0,1fr)_auto]"
      data-candidate={candidate.album_id}
    >
      <Checkbox
        aria-label={`Select ${candidate.title}`}
        checked={selected}
        disabled={!canRequest(candidate)}
        onCheckedChange={(value) => onSelect(value === true)}
      />
      <Link href={`/album?id=${encodeURIComponent(candidate.album_id)}`}>
        <ToolCover alt="" size={44} src={candidate.cover_url} />
      </Link>
      <div className="min-w-0">
        <Link
          className="block truncate text-sm font-medium text-zinc-100 hover:underline"
          href={`/album?id=${encodeURIComponent(candidate.album_id)}`}
        >
          {candidate.title}
        </Link>
        <p className="truncate text-xs text-zinc-500">
          {candidate.artist_name}
          {candidate.year ? ` · ${candidate.year}` : ""} ·{" "}
          {formatCount(candidate.track_count)} tracks
        </p>
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          <QualityBadge quality={candidate.quality} />
          {candidate.reasons.map((reason) => (
            <ReasonBadge key={reason.kind} reason={reason} />
          ))}
          {candidate.enhanced_tracks > 0 && (
            <span className="inline-flex h-5 items-center gap-1 rounded-full border border-quality-enhanced/40 px-2 text-[11px] font-medium text-quality-enhanced">
              <Sparkles className="h-3 w-3" />
              {candidate.enhanced_tracks} AI-enhanced
            </span>
          )}
        </div>
      </div>
      <div className="col-span-3 min-w-0 pl-7 sm:col-span-1 sm:pl-0">
        {request ? (
          <RequestStatus request={request} />
        ) : search ? (
          <p
            className={cn(
              "text-xs",
              search.found ? "text-quality-high" : "text-zinc-500",
            )}
            title={search.titles.join("\n") || undefined}
          >
            {search.found === null
              ? "FETCH answered; results unclear"
              : search.found
                ? `FETCH found ${formatCount(search.count)} result${search.count === 1 ? "" : "s"}`
                : "FETCH found nothing"}
          </p>
        ) : candidate.enhance_queued > 0 ? (
          <p className="flex items-center gap-1.5 text-xs text-quality-enhanced">
            <Loader2 className="h-3 w-3 animate-spin" />
            {candidate.enhance_queued} track(s) being AI-enhanced
          </p>
        ) : (
          <p className="text-xs text-zinc-600">Not requested</p>
        )}
      </div>
      <div className="col-span-3 flex flex-wrap justify-end gap-1.5 sm:col-span-1">
        {request && isUpgradeOpen(request.state) ? (
          <Button
            disabled={busy !== null}
            onClick={() =>
              run("cancel", async () => {
                await cancelUpgradeRequest(request.id);
                return "Request cancelled";
              })
            }
            size="sm"
            variant="ghost"
          >
            <X />
            Cancel
          </Button>
        ) : (
          <>
            {request &&
              ["failed", "cancelled", "not_found"].includes(request.state) && (
                <Button
                  disabled={busy !== null || !fetchEnabled}
                  onClick={() =>
                    run("retry", async () => {
                      await retryUpgradeRequest(request.id);
                      return "Asked FETCH again";
                    })
                  }
                  size="sm"
                  variant="ghost"
                >
                  <RotateCcw />
                  Retry
                </Button>
              )}
            {request && (
              <Button
                aria-label="Forget this request"
                disabled={busy !== null}
                onClick={() =>
                  run("delete", async () => {
                    await deleteUpgradeRequest(request.id);
                  })
                }
                size="sm"
                title="Forget this request"
                variant="ghost"
              >
                <Trash2 />
              </Button>
            )}
            <Button
              aria-label={`Check if FETCH has ${candidate.title}`}
              disabled={busy !== null || !fetchEnabled}
              onClick={() =>
                run("search", async () => {
                  const outcome = await searchFetchForAlbum(candidate.album_id);
                  return outcome.found === false
                    ? "FETCH found nothing for this album"
                    : undefined;
                })
              }
              size="sm"
              title="Search FETCH's sources first"
              variant="ghost"
            >
              {busy === "search" ? (
                <Loader2 className="animate-spin" />
              ) : (
                <Search />
              )}
            </Button>
            {enhancerEnabled && (
              <Button
                disabled={busy !== null}
                onClick={() =>
                  run("enhance", async () => {
                    const queued = await enhanceWithAi({
                      albumIds: [candidate.album_id],
                    });
                    return queued
                      ? `${queued} track(s) queued for AI enhancement`
                      : "Nothing to enhance: every track has a copy already";
                  })
                }
                size="sm"
                title="Make an AI-enhanced copy. The original stays."
                variant="ghost"
              >
                <Sparkles />
                Enhance
              </Button>
            )}
            <Button
              disabled={
                busy !== null || !fetchEnabled || !canRequest(candidate)
              }
              onClick={() => onRequest([candidate])}
              size="sm"
              variant="secondary"
            >
              <Download />
              Find lossless
            </Button>
          </>
        )}
      </div>
    </li>
  );
}

function RequestDialog({
  candidates,
  onClose,
  onDone,
}: {
  candidates: UpgradeCandidate[];
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [sending, setSending] = useState(false);
  const send = async () => {
    setSending(true);
    try {
      const made = await requestLosslessVersions(
        candidates.map((candidate) => candidate.album_id),
      );
      toast.success(
        made.length === 1
          ? "Asked FETCH for a lossless copy"
          : `Asked FETCH for ${made.length} lossless copies`,
      );
      await onDone();
      onClose();
    } catch (error) {
      toast(getApiErrorMessage(error, "FETCH could not be asked."));
    } finally {
      setSending(false);
    }
  };
  return (
    <Dialog onOpenChange={(open) => !open && onClose()} open>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {candidates.length === 1
              ? "Find a lossless copy?"
              : `Find lossless copies of ${candidates.length} albums?`}
          </DialogTitle>
          <DialogDescription>
            Mujin Music asks FETCH, on your network, to download each album as
            FLAC. When it arrives and checks out as real lossless, it becomes
            the version that plays; the copy you have now stays as an alternate
            version.
          </DialogDescription>
        </DialogHeader>
        <ul className="max-h-60 space-y-2 overflow-y-auto rounded-lg bg-black/30 p-3">
          {candidates.map((candidate) => (
            <li className="min-w-0" key={candidate.album_id}>
              <p className="truncate text-sm text-zinc-200">
                {candidate.title}{" "}
                <span className="text-zinc-500">· {candidate.artist_name}</span>
              </p>
              <p className="truncate font-mono text-[11px] text-zinc-500">
                “
                {fetchQueryPreview({
                  artist: candidate.artist_name,
                  title: candidate.title,
                  year: candidate.year,
                  musicbrainzId: candidate.musicbrainz_id,
                })}
                ”
              </p>
            </li>
          ))}
        </ul>
        <DialogFooter>
          <Button onClick={onClose} variant="ghost">
            Not now
          </Button>
          <Button disabled={sending} onClick={() => void send()}>
            {sending ? <Loader2 className="animate-spin" /> : <Download />}
            {candidates.length === 1
              ? "Ask FETCH"
              : `Ask FETCH for ${candidates.length}`}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function BetterVersionsPanel() {
  const queryClient = useQueryClient();
  const [filter, setFilter] = useState<CandidateFilter>("all");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirming, setConfirming] = useState<UpgradeCandidate[] | null>(null);
  const data = useQuery({
    queryKey: betterVersionsKey,
    queryFn: getUpgradeCandidates,
    refetchInterval: (query) =>
      query.state.data
        ? refreshInterval(
            query.state.data.candidates,
            query.state.data.requests,
          )
        : false,
  });
  const queue = useQuery({
    queryKey: [...betterVersionsKey, "enhance"],
    queryFn: getEnhanceQueue,
    enabled: data.data?.enhancer_enabled === true,
    refetchInterval: (query) =>
      (query.state.data?.counts.queued ?? 0) +
        (query.state.data?.counts.running ?? 0) >
      0
        ? 4_000
        : false,
  });
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: betterVersionsKey });
    await invalidateCatalogRevisionQueries(queryClient);
  };
  const candidates = useMemo(() => data.data?.candidates ?? [], [data.data]);
  const visible = candidates.filter((candidate) =>
    matchesFilter(candidate, filter),
  );
  const chosen = requestableSelection(candidates, selected);
  const fetchEnabled = data.data?.fetch_enabled === true;
  const enhancerEnabled = data.data?.enhancer_enabled === true;
  const check = data.data?.lossless_check ?? null;
  const counts = queue.data?.counts;
  const enhancing = (counts?.queued ?? 0) + (counts?.running ?? 0);

  const runAction = async (work: () => Promise<string>) => {
    try {
      toast.success(await work());
      await refresh();
    } catch (error) {
      toast(getApiErrorMessage(error, "That did not work."));
    }
  };

  return (
    <div className="space-y-6">
      <ToolCard
        action={
          <>
            <Button
              disabled={check?.state === "running"}
              onClick={() =>
                void runAction(async () => {
                  await checkForFakeLossless();
                  return "Checking lossless files for lossy sources";
                })
              }
              variant="secondary"
            >
              <AudioWaveform />
              Check for fake lossless
            </Button>
            <Button
              disabled={!fetchEnabled || chosen.length === 0}
              onClick={() => setConfirming(chosen)}
            >
              <Download />
              Request selected
              {chosen.length > 0 ? ` (${chosen.length})` : ""}
            </Button>
          </>
        }
        description={
          <>
            Albums whose best copy is lossy, low bitrate or a FLAC made from an
            MP3. Ask FETCH for a lossless copy: when it arrives and passes the
            same spectral check, it plays instead and the old copy stays as an
            alternate version.{" "}
            {!fetchEnabled && (
              <Link
                className="text-zinc-300 underline underline-offset-2"
                href="/settings?tab=better-versions"
              >
                Connect FETCH in Settings.
              </Link>
            )}
          </>
        }
        title={
          <span className="flex items-baseline gap-2">
            Better versions
            <span className="text-sm font-normal tabular-nums text-zinc-500">
              {formatCount(candidates.length)}
            </span>
            {data.data?.auto_request && (
              <span className="rounded-full bg-mujin-copper/12 px-2 text-[11px] font-medium text-mujin-copper">
                Requesting automatically
              </span>
            )}
          </span>
        }
      >
        {check && (
          <JobProgress
            compact
            job={check}
            onCancel={() => void cancelToolJob(check.id)}
          />
        )}
      </ToolCard>

      {enhancerEnabled && (
        <ToolCard
          action={
            <>
              {enhancing > 0 && (
                <Button
                  onClick={() =>
                    void runAction(async () => {
                      const stopped = await cancelEnhanceQueue();
                      return `Stopped ${stopped} track(s)`;
                    })
                  }
                  variant="ghost"
                >
                  <Ban />
                  Stop
                </Button>
              )}
              <Button
                onClick={() =>
                  void runAction(async () => {
                    const queued = await enhanceUnfoundWithAi();
                    return queued
                      ? `${queued} track(s) queued for AI enhancement`
                      : "Nothing to enhance: ask FETCH first, then enhance what it could not find";
                  })
                }
                variant="secondary"
              >
                <Sparkles />
                Enhance all lossy that FETCH couldn&apos;t find
              </Button>
            </>
          }
          className="border-quality-enhanced/15"
          description="Your enhancer (Apollo) generates plausible treble the lossy encode threw away. It is not the original master: copies are labelled AI-enhanced, the originals stay, and a real lossless copy always wins."
          title={
            <span className="flex items-center gap-2">
              <Sparkles className="h-4 w-4 text-quality-enhanced" />
              AI enhance
            </span>
          }
        >
          {counts ? (
            <p className="text-xs text-zinc-500" data-enhance-counts>
              {enhancing > 0
                ? `${counts.running ? "Enhancing" : "Waiting for the enhancer"} · ${counts.queued} queued`
                : "Nothing queued"}{" "}
              · {counts.done} done
              {counts.failed ? ` · ${counts.failed} failed` : ""}
            </p>
          ) : null}
          {queue.data?.jobs
            .filter((job) => job.state === "running")
            .map((job) => (
              <div className="mt-2" key={job.id}>
                <p className="truncate text-xs text-zinc-300">{job.title}</p>
                <div className="mt-1 h-1 overflow-hidden rounded-full bg-white/[0.06]">
                  <div
                    className="h-full rounded-full bg-quality-enhanced transition-[width] duration-500"
                    style={{ width: `${Math.max(2, job.progress * 100)}%` }}
                  />
                </div>
              </div>
            ))}
        </ToolCard>
      )}

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div
          aria-label="Show"
          className="flex rounded-full bg-white/[0.05] p-0.5"
          role="radiogroup"
        >
          {(Object.keys(candidateFilterLabels) as CandidateFilter[]).map(
            (key) => (
              <button
                aria-checked={filter === key}
                className={cn(
                  "h-7 rounded-full px-3 text-xs font-medium transition-colors",
                  filter === key
                    ? "bg-white text-black"
                    : "text-zinc-400 hover:text-white",
                )}
                key={key}
                onClick={() => setFilter(key)}
                role="radio"
                type="button"
              >
                {candidateFilterLabels[key]}
              </button>
            ),
          )}
        </div>
        <div className="flex items-center gap-2 text-xs text-zinc-500">
          {visible.length > 0 && (
            <button
              className="hover:text-white"
              onClick={() =>
                setSelected(
                  new Set(
                    visible
                      .filter(canRequest)
                      .map((candidate) => candidate.album_id),
                  ),
                )
              }
              type="button"
            >
              Select all
            </button>
          )}
          {selected.size > 0 && (
            <button
              className="hover:text-white"
              onClick={() => setSelected(new Set())}
              type="button"
            >
              Clear
            </button>
          )}
          <Link
            className="inline-flex items-center gap-1 hover:text-white"
            href="/settings?tab=better-versions"
          >
            <Settings2 className="h-3.5 w-3.5" />
            Settings
          </Link>
        </div>
      </div>

      {data.isPending ? (
        <div className="flex h-40 items-center justify-center text-zinc-500">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      ) : data.isError ? (
        <EmptyState icon={<Ban />} title="Better versions could not load">
          {getApiErrorMessage(data.error, "Try again in a moment.")}
        </EmptyState>
      ) : visible.length === 0 ? (
        <EmptyState icon={<Sparkles />} title="Nothing to upgrade here">
          {filter === "all"
            ? "Every album already plays from a genuine lossless copy. Run “Check for fake lossless” to look for FLACs made from MP3s."
            : "No album matches this view."}
        </EmptyState>
      ) : (
        <ul className="rounded-xl border border-white/[0.07] bg-white/[0.015] px-3">
          {visible.map((candidate) => (
            <CandidateRow
              candidate={candidate}
              enhancerEnabled={enhancerEnabled}
              fetchEnabled={fetchEnabled}
              key={candidate.album_id}
              onChanged={refresh}
              onRequest={setConfirming}
              onSelect={(value) =>
                setSelected((current) => {
                  const next = new Set(current);
                  if (value) next.add(candidate.album_id);
                  else next.delete(candidate.album_id);
                  return next;
                })
              }
              selected={selected.has(candidate.album_id)}
            />
          ))}
        </ul>
      )}

      {confirming && (
        <RequestDialog
          candidates={confirming}
          onClose={() => setConfirming(null)}
          onDone={async () => {
            setSelected(new Set());
            await refresh();
          }}
        />
      )}
    </div>
  );
}
