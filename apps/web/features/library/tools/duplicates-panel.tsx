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
import { songMenuQueueItem } from "@/features/library/song-menu-state";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { cn } from "@/lib/cn";
import {
  getDuplicates,
  preferTrackCopy,
  reanalyzeLibraryItems,
  resolveDuplicate,
  resolveSuggestedDuplicates,
  restoreHiddenCopies,
  type DuplicateGroup,
  type MultiCopyTrack,
  type ToolJob,
  type ToolSong,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AudioWaveform,
  ChevronDown,
  CopyCheck,
  Crown,
  EyeOff,
  Fingerprint,
  Loader2,
  Merge,
  Play,
  Sparkles,
  Undo2,
  X,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import { getApiErrorMessage } from "./tool-errors";
import {
  EmptyState,
  QualityBadge,
  ToolCard,
  ToolCover,
} from "./tool-primitives";
import {
  duplicateEvidenceLabel,
  duplicateKindLabel,
  formatBytes,
  formatCount,
  formatTrackTime,
  percent,
  runningJob,
} from "./tools-model";
import { useStartToolJob } from "./use-tool-jobs";

const duplicatesKey = ["library-tools", "duplicates"] as const;

export function DuplicatesPanel({ jobs }: { jobs: ToolJob[] }) {
  const queryClient = useQueryClient();
  const startJob = useStartToolJob();
  const duplicates = useQuery({
    queryKey: duplicatesKey,
    queryFn: getDuplicates,
  });
  const [confirmBulk, setConfirmBulk] = useState(false);
  const [bulkBusy, setBulkBusy] = useState(false);
  const [showHidden, setShowHidden] = useState(false);
  const analyzing = runningJob(jobs, "reanalyze");

  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
    await invalidateCatalogRevisionQueries(queryClient);
  };

  const data = duplicates.data;
  const suggested = data?.groups.filter((group) => group.suggested) ?? [];
  const review = data?.groups.filter((group) => !group.suggested) ?? [];
  const suggestedCopies = suggested.reduce(
    (sum, group) => sum + group.members.length - 1,
    0,
  );

  const keepBest = async () => {
    setBulkBusy(true);
    try {
      const result = await resolveSuggestedDuplicates("merge");
      toast.success(
        `Kept the best copy in ${formatCount(result.groups)} group${result.groups === 1 ? "" : "s"}`,
      );
      setConfirmBulk(false);
      await refresh();
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not tidy the duplicates."));
    } finally {
      setBulkBusy(false);
    }
  };

  if (duplicates.isPending) {
    return (
      <div className="flex h-48 items-center justify-center text-zinc-500">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    );
  }
  if (!data) {
    return (
      <EmptyState icon={<CopyCheck />} title="Duplicates could not be loaded">
        Try again in a moment.
      </EmptyState>
    );
  }
  const printed = data.fingerprints;

  return (
    <div className="space-y-6">
      <ToolCard
        action={
          suggested.length > 0 ? (
            <Button onClick={() => setConfirmBulk(true)}>
              <Sparkles />
              Keep best copies ({formatCount(suggestedCopies)})
            </Button>
          ) : undefined
        }
        description="The same recording held more than once: a FLAC rip and an MP3 download, a loose single next to its album, or a copy under different tags. Parson compares titles, lengths, ISRCs and the sound itself."
        title={
          <span className="flex items-baseline gap-2">
            Duplicates
            <span className="text-sm font-normal tabular-nums text-zinc-500">
              {formatCount(data.groups.length)}
            </span>
          </span>
        }
      >
        <div className="flex flex-wrap items-center gap-4 rounded-lg border border-white/[0.06] bg-black/30 px-4 py-3">
          <Fingerprint className="h-5 w-5 shrink-0 text-[#b26bff]" />
          <div className="min-w-0 flex-1">
            <p className="text-sm text-zinc-200">
              Acoustic fingerprints for{" "}
              <span className="font-semibold tabular-nums">
                {formatCount(printed.tracks)}
              </span>{" "}
              of {formatCount(printed.total)} tracks
              {printed.method && (
                <span className="text-zinc-500">
                  {" "}
                  ·{" "}
                  {printed.method === "chromaprint"
                    ? "Chromaprint"
                    : "loudness envelope"}
                </span>
              )}
            </p>
            <div className="mt-2 h-1 overflow-hidden rounded-full bg-white/[0.05]">
              <div
                className="h-full rounded-full bg-[#b26bff]"
                style={{
                  width: `${percent(printed.tracks, printed.total)}%`,
                }}
              />
            </div>
          </div>
          <Button
            disabled={Boolean(analyzing)}
            onClick={() =>
              void startJob(
                () => reanalyzeLibraryItems({ scope: "library" }),
                "Could not start the analysis.",
              )
            }
            size="sm"
            variant="secondary"
          >
            {analyzing ? (
              <Loader2 className="animate-spin" />
            ) : (
              <AudioWaveform />
            )}
            {analyzing ? "Analyzing…" : "Fingerprint library"}
          </Button>
        </div>
      </ToolCard>

      {data.groups.length === 0 && data.copies.length === 0 ? (
        <EmptyState icon={<CopyCheck />} title="No duplicates">
          Every recording appears once.
          {data.dismissed > 0 &&
            ` ${formatCount(data.dismissed)} group(s) marked as not duplicates.`}
        </EmptyState>
      ) : null}

      {suggested.length > 0 && (
        <GroupSection
          description="Safe to tidy: copies of the same album, or loose tracks that copy an album track."
          groups={suggested}
          onChanged={refresh}
          title="Ready to tidy"
        />
      )}
      {review.length > 0 && (
        <GroupSection
          description="The same recording on different albums, such as a compilation. Usually worth keeping; decide one by one."
          groups={review}
          onChanged={refresh}
          title="Review"
        />
      )}

      {data.copies.length > 0 && (
        <CopiesSection copies={data.copies} onChanged={refresh} />
      )}

      {data.hidden.length > 0 && (
        <section className="rounded-xl border border-white/[0.07]">
          <button
            aria-expanded={showHidden}
            className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left"
            onClick={() => setShowHidden(!showHidden)}
            type="button"
          >
            <span>
              <span className="text-sm font-semibold text-white">
                Hidden and merged copies
              </span>
              <span className="ml-2 text-sm tabular-nums text-zinc-500">
                {formatCount(data.hidden.length)}
              </span>
              <span className="block text-xs text-zinc-500">
                Still on disk, kept out of your library. Restore any of them.
              </span>
            </span>
            <ChevronDown
              className={cn(
                "h-4 w-4 text-zinc-500 transition-transform",
                showHidden && "rotate-180",
              )}
            />
          </button>
          {showHidden && (
            <ul className="divide-y divide-white/[0.05] border-t border-white/[0.06]">
              {data.hidden.map((copy) => (
                <li
                  className="flex items-center gap-3 px-4 py-2.5"
                  key={copy.path}
                >
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm text-zinc-200">
                      {copy.title}
                      <span className="ml-2 text-xs text-zinc-500">
                        {copy.state === "merged" ? "Merged" : "Hidden"}
                        {copy.kept ? ` into ${copy.kept.album}` : ""}
                      </span>
                    </p>
                    <p className="truncate font-mono text-[11px] text-zinc-600">
                      {copy.path}
                    </p>
                  </div>
                  <Button
                    onClick={() =>
                      void restoreHiddenCopies([copy.path])
                        .then(refresh)
                        .then(() => toast.success("Copy restored"))
                        .catch((error: unknown) =>
                          toast(
                            getApiErrorMessage(error, "Could not restore it."),
                          ),
                        )
                    }
                    size="sm"
                    variant="ghost"
                  >
                    <Undo2 />
                    Restore
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      <Dialog onOpenChange={setConfirmBulk} open={confirmBulk}>
        <DialogContent className="[&>*]:min-w-0">
          <DialogHeader>
            <DialogTitle>Keep the best copy of each?</DialogTitle>
            <DialogDescription>
              In {formatCount(suggested.length)} group
              {suggested.length === 1 ? "" : "s"}, Parson keeps the highest
              quality copy and merges the other {formatCount(suggestedCopies)}{" "}
              into it: likes and playlist entries move to the kept copy. Nothing
              is deleted from disk, and any copy can be restored later.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button onClick={() => setConfirmBulk(false)} variant="ghost">
              Cancel
            </Button>
            <Button disabled={bulkBusy} onClick={() => void keepBest()}>
              {bulkBusy ? <Loader2 className="animate-spin" /> : <Merge />}
              Keep best copies
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function GroupSection({
  title,
  description,
  groups,
  onChanged,
}: {
  title: string;
  description: string;
  groups: DuplicateGroup[];
  onChanged: () => Promise<void>;
}) {
  const [limit, setLimit] = useState(40);
  return (
    <section>
      <div className="mb-3">
        <h2 className="text-[15px] font-semibold text-white">
          {title}
          <span className="ml-2 text-sm font-normal tabular-nums text-zinc-500">
            {formatCount(groups.length)}
          </span>
        </h2>
        <p className="text-sm text-zinc-500">{description}</p>
      </div>
      <div className="space-y-3">
        {groups.slice(0, limit).map((group) => (
          <GroupCard group={group} key={group.key} onChanged={onChanged} />
        ))}
      </div>
      {groups.length > limit && (
        <Button
          className="mt-3"
          onClick={() => setLimit(limit + 40)}
          variant="ghost"
        >
          Show more
        </Button>
      )}
    </section>
  );
}

function GroupCard({
  group,
  onChanged,
}: {
  group: DuplicateGroup;
  onChanged: () => Promise<void>;
}) {
  const playback = useZonePlayback();
  const [keep, setKeep] = useState(group.best);
  const [busy, setBusy] = useState<string | null>(null);
  const first = group.songs[0];
  if (!first) return null;

  const act = async (action: "merge" | "hide" | "dismiss", message: string) => {
    setBusy(action);
    try {
      await resolveDuplicate(group.key, action, {
        keepSongId: keep,
        songIds:
          action === "hide"
            ? group.songs
                .filter((song) => song.id !== keep)
                .map((song) => song.id)
            : undefined,
      });
      toast.success(message);
      await onChanged();
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not update this group."));
    } finally {
      setBusy(null);
    }
  };

  const play = (song: ToolSong) =>
    void playback.playQueue([
      songMenuQueueItem({
        songId: song.id,
        songName: song.title,
        artistId: song.artist_id,
        artistName: song.artist,
        albumId: song.album_id,
        albumName: song.album,
        albumCover: song.cover_url,
      }),
    ]);

  return (
    <article className="overflow-hidden rounded-xl border border-white/[0.07] bg-white/[0.02]">
      <header className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-white/[0.05] px-4 py-3">
        <h3 className="min-w-0 truncate text-sm font-semibold text-white">
          {first.title}
          <span className="font-normal text-zinc-500"> · {first.artist}</span>
        </h3>
        <span className="rounded-full bg-white/[0.05] px-2 py-0.5 text-[11px] text-zinc-400">
          {duplicateKindLabel(group.kind)}
        </span>
        <span
          className={cn(
            "flex items-center gap-1 text-[11px]",
            group.confidence === "high" ? "text-[#3ddc97]" : "text-zinc-500",
          )}
        >
          {group.evidence.fingerprint_similarity !== null && (
            <Fingerprint className="h-3 w-3" />
          )}
          {duplicateEvidenceLabel(group)}
        </span>
      </header>
      <ul role="radiogroup" aria-label={`Copy of ${first.title} to keep`}>
        {group.songs.map((song) => {
          const kept = keep === song.id;
          return (
            <li
              className={cn(
                "flex items-center gap-3 px-4 py-2.5 transition-colors",
                kept ? "bg-[#b26bff]/[0.06]" : "hover:bg-white/[0.02]",
              )}
              key={song.id}
            >
              <button
                aria-checked={kept}
                aria-label={`Keep the copy on ${song.album}`}
                className={cn(
                  "flex h-4 w-4 shrink-0 items-center justify-center rounded-full border",
                  kept ? "border-[#b26bff] bg-[#b26bff]" : "border-zinc-600",
                )}
                onClick={() => setKeep(song.id)}
                role="radio"
                type="button"
              >
                {kept && <span className="h-1.5 w-1.5 rounded-full bg-black" />}
              </button>
              <ToolCover alt={song.album} size={40} src={song.cover_url} />
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-2 truncate text-sm text-zinc-100">
                  <span className="truncate">{song.album}</span>
                  {song.id === group.best && (
                    <span className="flex shrink-0 items-center gap-1 text-[11px] text-[#d6b8ff]">
                      <Crown className="h-3 w-3" />
                      Best
                    </span>
                  )}
                </p>
                <p
                  className="truncate font-mono text-[11px] text-zinc-600"
                  title={song.path}
                >
                  {song.path}
                </p>
              </div>
              <QualityBadge emphasis={kept} quality={song.quality} />
              <span className="hidden w-12 shrink-0 text-right text-xs tabular-nums text-zinc-500 sm:block">
                {formatTrackTime(song.duration)}
              </span>
              <button
                aria-label={`Play the copy on ${song.album}`}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-zinc-400 transition-colors hover:bg-white/8 hover:text-white"
                onClick={() => play(song)}
                title="Listen to this copy"
                type="button"
              >
                <Play className="h-4 w-4" />
              </button>
            </li>
          );
        })}
      </ul>
      <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-white/[0.05] px-4 py-2.5">
        <Button
          disabled={busy !== null}
          onClick={() => void act("dismiss", "Marked as not duplicates")}
          size="sm"
          variant="ghost"
        >
          <X />
          Not duplicates
        </Button>
        <Button
          disabled={busy !== null}
          onClick={() => void act("hide", "Other copies hidden")}
          size="sm"
          variant="ghost"
        >
          {busy === "hide" ? <Loader2 className="animate-spin" /> : <EyeOff />}
          Hide others
        </Button>
        <Button
          disabled={busy !== null}
          onClick={() =>
            void act(
              "merge",
              "Merged: likes and playlists follow the kept copy",
            )
          }
          size="sm"
        >
          {busy === "merge" ? <Loader2 className="animate-spin" /> : <Merge />}
          Keep selected
        </Button>
      </footer>
    </article>
  );
}

function CopiesSection({
  copies,
  onChanged,
}: {
  copies: MultiCopyTrack[];
  onChanged: () => Promise<void>;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [limit, setLimit] = useState(30);
  const prefer = async (trackId: string, path: string | null) => {
    setBusy(trackId);
    try {
      await preferTrackCopy(trackId, path);
      toast.success(
        path ? "This copy plays from now on" : "Back to the best copy",
      );
      await onChanged();
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not change the copy."));
    } finally {
      setBusy(null);
    }
  };
  return (
    <section>
      <div className="mb-3">
        <h2 className="text-[15px] font-semibold text-white">
          One track, several files
          <span className="ml-2 text-sm font-normal tabular-nums text-zinc-500">
            {formatCount(copies.length)}
          </span>
        </h2>
        <p className="text-sm text-zinc-500">
          Parson already plays the highest quality file of these tracks. Pick
          another copy if you prefer its mastering.
        </p>
      </div>
      <div className="overflow-hidden rounded-xl border border-white/[0.07]">
        <ul className="divide-y divide-white/[0.05]">
          {copies.slice(0, limit).map((copy) => (
            <li className="px-4 py-3" key={copy.song.id}>
              <div className="flex items-center gap-3">
                <ToolCover
                  alt={copy.song.album}
                  size={36}
                  src={copy.song.cover_url}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm text-zinc-100">
                    {copy.song.title}
                    <span className="text-zinc-500"> · {copy.song.album}</span>
                  </p>
                </div>
                {copy.chosen && (
                  <Button
                    disabled={busy === copy.song.id}
                    onClick={() => void prefer(copy.song.id, null)}
                    size="sm"
                    variant="ghost"
                  >
                    <Undo2 />
                    Use best
                  </Button>
                )}
              </div>
              <div className="ml-12 mt-2 grid gap-1.5">
                {copy.files.map((file) => (
                  <button
                    aria-pressed={file.playing}
                    className={cn(
                      "flex items-center gap-3 rounded-md border px-3 py-1.5 text-left transition-colors",
                      file.playing
                        ? "border-[#b26bff]/40 bg-[#b26bff]/[0.07]"
                        : "border-white/[0.06] hover:bg-white/[0.04]",
                    )}
                    disabled={busy === copy.song.id || file.playing}
                    key={file.path}
                    onClick={() => void prefer(copy.song.id, file.path)}
                    type="button"
                  >
                    <QualityBadge quality={file.quality} />
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-500">
                      {file.path}
                    </span>
                    <span className="shrink-0 text-[11px] tabular-nums text-zinc-600">
                      {formatBytes(file.size_bytes)}
                    </span>
                    <span
                      className={cn(
                        "w-14 shrink-0 text-right text-[11px]",
                        file.playing ? "text-[#d6b8ff]" : "text-zinc-600",
                      )}
                    >
                      {file.playing ? "Playing" : "Use"}
                    </span>
                  </button>
                ))}
              </div>
            </li>
          ))}
        </ul>
      </div>
      {copies.length > limit && (
        <Button
          className="mt-3"
          onClick={() => setLimit(limit + 30)}
          variant="ghost"
        >
          Show more
        </Button>
      )}
    </section>
  );
}
