"use client";

import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import { defaultCover } from "@/lib/images/default-cover";
import {
  getFocusCompositions,
  getFocusPage,
  isFocused,
  withEntity,
  workDescription,
  workScope,
  type CompositionRecording,
  type FocusQuery,
  type FocusWork,
} from "@parson/music-sdk";
import {
  keepPreviousData,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ChevronDown, Play } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";
import {
  focusCountLabel,
  focusQueryKey,
  recordingQueue,
} from "./focus/focus-state";
import { usePlayQueue } from "./focus/use-play-focus";
import { uniqueById } from "./library-items";
import {
  InfiniteLoad,
  LibraryLoading,
  LibraryMessage,
  formatDuration,
} from "./library-view-primitives";

const PAGE_SIZE = 100;

/** The focus that lists one work's focused recordings. */
export function workRecordingsFocus(
  focus: FocusQuery,
  workId: string,
): FocusQuery {
  return {
    ...withEntity(focus, "songs"),
    sort: null,
    direction: null,
    scope: [...(focus.scope ?? []), workScope(workId)],
  };
}

/**
 * The Compositions browser: works with their form, scoring and date, as in
 * Roon. Rows expand into the recordings the focus matches; play starts the
 * first recording in movement order.
 */
export default function CompositionList({
  composerId,
  emptyBody,
  focus,
  onClear,
}: {
  /** On a composer page, other composers are listed as co-writers. */
  composerId?: string;
  emptyBody: string;
  focus: FocusQuery;
  onClear: () => void;
}) {
  const pages = useInfiniteQuery({
    queryKey: ["library", "focus", "page", focusQueryKey(focus)],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => getFocusPage(focus, pageParam, PAGE_SIZE),
    getNextPageParam: (lastPage, all) => {
      const next = all.length * PAGE_SIZE;
      return next < lastPage.total ? next : undefined;
    },
    placeholderData: keepPreviousData,
  });
  const [open, setOpen] = useState<string | null>(null);
  const works = uniqueById(
    pages.data?.pages.flatMap((page) => page.compositions ?? []) ?? [],
  );
  const last = pages.data?.pages[pages.data.pages.length - 1];
  if (pages.isPending && !pages.data) return <LibraryLoading compact />;
  if (pages.isError && !pages.data) {
    return (
      <LibraryMessage
        action="Try again"
        onAction={() => void pages.refetch()}
        title="Compositions could not be loaded"
      />
    );
  }
  if (!works.length && isFocused(focus)) {
    return (
      <LibraryMessage
        action="Clear focus"
        body="No composition matches every filter. Remove a filter or switch to Match any."
        onAction={onClear}
        title="No matches"
      />
    );
  }
  if (!works.length) {
    return <LibraryMessage body={emptyBody} title="No compositions" />;
  }
  return (
    <div className="space-y-4">
      {composerId && (
        <p aria-live="polite" className="text-sm tabular-nums text-zinc-500">
          {focusCountLabel(focus, last?.total, last?.libraryTotal)}
        </p>
      )}
      <ul className="overflow-hidden rounded-xl border border-white/8">
        {works.map((work) => (
          <WorkRow
            composerId={composerId}
            expanded={open === work.id}
            focus={focus}
            key={work.id}
            onToggle={() =>
              setOpen((current) => (current === work.id ? null : work.id))
            }
            work={work}
          />
        ))}
      </ul>
      <InfiniteLoad
        hasMore={pages.hasNextPage}
        loading={pages.isFetchingNextPage}
        onLoadMore={() => void pages.fetchNextPage()}
      />
    </div>
  );
}

function WorkRow({
  composerId,
  expanded,
  focus,
  onToggle,
  work,
}: {
  composerId?: string;
  expanded: boolean;
  focus: FocusQuery;
  onToggle: () => void;
  work: FocusWork;
}) {
  const playQueue = usePlayQueue();
  const client = useQueryClient();
  const recordingsFocus = workRecordingsFocus(focus, work.id);
  const recordingsKey = [
    "library",
    "focus",
    "compositions",
    focusQueryKey(recordingsFocus),
  ];
  const description = workDescription(work);
  const composers = work.composers.filter(
    (composer) => composer.id !== composerId,
  );
  const recordings = `${work.recordingCount.toLocaleString("en-US")} ${
    work.recordingCount === 1 ? "recording" : "recordings"
  }`;

  const play = async () => {
    try {
      const data = await client.fetchQuery({
        queryKey: recordingsKey,
        queryFn: () => getFocusCompositions(recordingsFocus),
      });
      const first = data.compositions[0]?.recordings[0];
      if (first) playQueue(recordingQueue(first), false);
      else toast("Nothing to play in this composition.");
    } catch {
      toast("Could not play this composition.");
    }
  };

  return (
    <li className="border-b border-white/6 last:border-0">
      <div className="flex items-center gap-2 px-3 py-2 hover:bg-white/[0.035] sm:px-4">
        <button
          aria-label={`Play ${work.title}`}
          className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
          onClick={() => void play()}
          type="button"
        >
          <Play className="h-4 w-4 fill-current" />
        </button>
        <button
          aria-expanded={expanded}
          className="flex min-w-0 flex-1 items-center gap-3 py-1.5 text-left"
          onClick={onToggle}
          type="button"
        >
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-semibold text-zinc-100">
              {work.title}
            </span>
            <span className="mt-0.5 block truncate text-xs text-zinc-400">
              {[
                composers.length
                  ? composers.map((composer) => composer.name).join(", ")
                  : null,
                description || null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
            <span className="mt-0.5 block truncate text-xs text-zinc-600">
              {[recordings, work.period].filter(Boolean).join(" · ")}
            </span>
          </span>
          {work.form && (
            <span className="hidden shrink-0 rounded-full border border-white/10 px-2 py-0.5 text-[11px] text-zinc-400 md:inline">
              {work.form}
            </span>
          )}
          <ChevronDown
            className={cn(
              "h-4 w-4 shrink-0 text-zinc-500 transition-transform",
              expanded && "rotate-180",
            )}
          />
        </button>
      </div>
      {expanded && (
        <WorkRecordings focus={recordingsFocus} queryKey={recordingsKey} />
      )}
    </li>
  );
}

function WorkRecordings({
  focus,
  queryKey,
}: {
  focus: FocusQuery;
  queryKey: unknown[];
}) {
  const playQueue = usePlayQueue();
  const data = useQuery({
    queryKey,
    queryFn: () => getFocusCompositions(focus),
  });
  const composition = data.data?.compositions[0];
  return (
    <div className="space-y-3 border-t border-white/6 bg-black/20 px-3 py-3 sm:px-4">
      {data.isPending && (
        <p className="py-2 text-xs text-zinc-500">Loading recordings…</p>
      )}
      {data.isError && (
        <p className="py-2 text-xs text-zinc-500">
          Recordings could not be loaded.
        </p>
      )}
      {composition?.recordings.map((recording, index) => (
        <RecordingRow
          key={`${recording.albumId ?? "loose"}-${index}`}
          onPlay={() => playQueue(recordingQueue(recording), false)}
          recording={recording}
        />
      ))}
    </div>
  );
}

export function RecordingRow({
  onPlay,
  recording,
}: {
  onPlay: () => void;
  recording: CompositionRecording;
}) {
  const cover = recording.coverPath
    ? `${getBaseURL()}/media/images/${encodeURIComponent(recording.coverPath)}`
    : defaultCover;
  const movements = recording.tracks.length > 1;
  return (
    <div className="rounded-lg border border-white/6 bg-white/[0.02] p-3">
      <div className="flex items-center gap-3">
        <div className="relative h-12 w-12 shrink-0 overflow-hidden rounded-md bg-zinc-900">
          <Image
            alt=""
            className="object-cover"
            fill
            sizes="48px"
            src={cover}
          />
        </div>
        <div className="min-w-0 flex-1">
          {recording.albumId ? (
            <Link
              className="block truncate text-sm font-medium text-zinc-100 hover:underline"
              href={`/album?id=${recording.albumId}`}
            >
              {recording.albumName}
            </Link>
          ) : (
            <span className="block truncate text-sm text-zinc-300">
              {recording.albumName ?? "Single track"}
            </span>
          )}
          <p className="truncate text-xs text-zinc-500">
            {[
              recording.performers.length
                ? recording.performers.join(", ")
                : recording.artistName,
              recording.releaseYear,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        </div>
        <span className="text-xs tabular-nums text-zinc-500">
          {formatDuration(recording.durationSeconds)}
        </span>
        <button
          aria-label={`Play ${recording.albumName ?? "recording"}`}
          className="grid h-8 w-8 shrink-0 place-items-center rounded-full bg-white text-black hover:bg-zinc-200"
          onClick={onPlay}
          type="button"
        >
          <Play className="h-3.5 w-3.5 fill-current" />
        </button>
      </div>
      {movements && (
        <ol className="mt-2 space-y-0.5 pl-15 text-xs text-zinc-400">
          {recording.tracks.map((track, index) => (
            <li className="flex gap-3" key={track.id}>
              <span className="w-5 shrink-0 text-right tabular-nums text-zinc-600">
                {track.movementNumber ?? index + 1}
              </span>
              <span className="min-w-0 flex-1 truncate">
                {track.movement ?? track.title}
              </span>
              <span className="tabular-nums text-zinc-600">
                {formatDuration(track.durationSeconds)}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
