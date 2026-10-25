"use client";

import { usePageTitle } from "@/components/app/title-metadata";
import AlbumCard from "@/features/library/album-card";
import {
  composerInitials,
  composerSummary,
  personRoles,
} from "@/features/library/composer-summary";
import EntityPageState from "@/features/library/entity-page-state";
import FocusBar from "@/features/library/focus/focus-bar";
import FocusPanel from "@/features/library/focus/focus-panel";
import {
  compositionSummary,
  focusCountLabel,
  focusQueryKey,
  qualityBadgeLabel,
  recordingQueue,
} from "@/features/library/focus/focus-state";
import SmartPlaylistDialog from "@/features/library/focus/smart-playlist-dialog";
import {
  usePlayFocus,
  usePlayQueue,
} from "@/features/library/focus/use-play-focus";
import { uniqueById } from "@/features/library/library-items";
import {
  InfiniteLoad,
  LibraryMessage,
  formatDuration,
} from "@/features/library/library-view-primitives";
import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import { defaultCover } from "@/lib/images/default-cover";
import {
  clearedFocus,
  composerScope,
  getFocusCompositions,
  getFocusPage,
  getFocusPerson,
  isFocused,
  scopedFocus,
  withEntity,
  type Composition,
  type CompositionRecording,
  type FocusQuery,
} from "@parson/music-sdk";
import {
  keepPreviousData,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { ChevronDown, Disc3, Feather, ListMusic, Play } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useMemo, useState } from "react";

const PAGE_SIZE = 100;
type ComposerTab = "discography" | "compositions";

function entityFor(tab: ComposerTab) {
  return tab === "compositions" ? "songs" : "albums";
}

export default function ComposerDetails() {
  const searchParams = useSearchParams();
  const id = searchParams?.get("id") ?? "";
  const requestedTab: ComposerTab =
    searchParams?.get("tab") === "compositions"
      ? "compositions"
      : "discography";
  const [tab, setTab] = useState<ComposerTab>(requestedTab);
  const scope = useMemo(() => (id ? [composerScope(id)] : []), [id]);
  const [focus, setFocus] = useState<FocusQuery>(() =>
    scopedFocus(entityFor(requestedTab), scope),
  );
  const [panelOpen, setPanelOpen] = useState(false);
  const [smartOpen, setSmartOpen] = useState(false);
  const playFocus = usePlayFocus();

  useEffect(() => {
    // A different composer starts from an empty focus.
    setFocus((current) =>
      current.scope === scope
        ? current
        : scopedFocus(entityFor(requestedTab), scope),
    );
  }, [scope, requestedTab]);

  const person = useQuery({
    queryKey: ["library", "focus", "person", id],
    queryFn: () => getFocusPerson(id),
    enabled: Boolean(id),
  });
  usePageTitle(person.data?.name);

  const selectTab = (next: ComposerTab) => {
    setTab(next);
    setFocus((current) => withEntity(current, entityFor(next)));
    const params = new URLSearchParams(window.location.search);
    if (next === "compositions") params.set("tab", next);
    else params.delete("tab");
    window.history.replaceState(
      window.history.state,
      "",
      `/composer?${params}`,
    );
  };

  if (!id) return <EntityPageState kind="composer" />;
  if (person.isPending) return <EntityPageState kind="composer" loading />;
  if (person.isError || !person.data) {
    return (
      <EntityPageState kind="composer" onRetry={() => void person.refetch()} />
    );
  }
  const composer = person.data.composer;

  return (
    <section className="mx-auto w-full max-w-266 space-y-7">
      <header className="flex items-center gap-5 pt-2">
        <div className="grid h-24 w-24 shrink-0 place-items-center rounded-full bg-linear-to-br from-amber-200/20 to-white/[0.04] text-3xl font-semibold text-amber-100 sm:h-28 sm:w-28">
          {composerInitials(person.data.name)}
        </div>
        <div className="min-w-0">
          <p className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-[0.14em] text-zinc-500">
            <Feather className="h-3.5 w-3.5" /> Composer
          </p>
          <h1 className="mt-1 truncate text-3xl font-semibold text-white sm:text-4xl">
            {person.data.name}
          </h1>
          <p className="mt-1.5 text-sm text-zinc-400">
            {[
              composer ? composerSummary(composer) : null,
              personRoles(person.data),
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        </div>
      </header>

      <div
        aria-label="Composer views"
        className="flex min-h-11 items-end gap-1 border-b border-white/8"
        role="tablist"
      >
        <ComposerTabButton
          active={tab === "discography"}
          icon={<Disc3 />}
          label="Discography"
          onClick={() => selectTab("discography")}
        />
        <ComposerTabButton
          active={tab === "compositions"}
          icon={<ListMusic />}
          label="Compositions"
          onClick={() => selectTab("compositions")}
        />
      </div>

      <div className="space-y-4">
        <FocusBar
          bookmarks={false}
          canPlay
          onCreateSmartPlaylist={() => setSmartOpen(true)}
          onPlay={(shuffle) => void playFocus(focus, shuffle)}
          onTogglePanel={() => setPanelOpen((open) => !open)}
          panelOpen={panelOpen}
          query={focus}
          setQuery={setFocus}
        />
        {panelOpen && <FocusPanel query={focus} setQuery={setFocus} />}
      </div>
      <SmartPlaylistDialog
        onOpenChange={setSmartOpen}
        open={smartOpen}
        query={focus}
      />

      {tab === "discography" ? (
        <Discography
          focus={focus}
          onClear={() => setFocus(clearedFocus(focus))}
        />
      ) : (
        <Compositions
          composerId={id}
          focus={focus}
          onClear={() => setFocus(clearedFocus(focus))}
        />
      )}
    </section>
  );
}

function ComposerTabButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-selected={active}
      className={cn(
        "relative flex h-11 shrink-0 items-center gap-2 px-4 text-sm font-medium transition-colors [&_svg]:h-4 [&_svg]:w-4",
        active ? "text-white" : "text-zinc-500 hover:text-zinc-200",
      )}
      onClick={onClick}
      role="tab"
      type="button"
    >
      {icon}
      {label}
      {active && (
        <span className="absolute inset-x-3 bottom-0 h-0.5 bg-white" />
      )}
    </button>
  );
}

function FocusEmpty({ onClear }: { onClear: () => void }) {
  return (
    <LibraryMessage
      action="Clear focus"
      body="Nothing by this composer matches every filter."
      onAction={onClear}
      title="No matches"
    />
  );
}

function Discography({
  focus,
  onClear,
}: {
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
  const albums = uniqueById(
    pages.data?.pages.flatMap((page) => page.albums) ?? [],
  );
  const last = pages.data?.pages[pages.data.pages.length - 1];
  if (pages.isPending && !pages.data) return null;
  if (!albums.length && isFocused(focus))
    return <FocusEmpty onClear={onClear} />;
  if (!albums.length) {
    return (
      <LibraryMessage
        title="No albums"
        body="No album credits this composer yet."
      />
    );
  }
  return (
    <div className="space-y-4">
      <p aria-live="polite" className="text-sm tabular-nums text-zinc-500">
        {focusCountLabel(focus, last?.total, last?.libraryTotal)}
      </p>
      <div className="grid grid-cols-2 gap-x-4 gap-y-10 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        {albums.map((album) => {
          const badge =
            album.quality === "hires" ? qualityBadgeLabel("hires") : null;
          return (
            <div className="relative min-w-0" key={album.id}>
              <AlbumCard
                album_cover={album.coverPath}
                album_id={album.id}
                album_name={album.name}
                artist_id={album.artistId}
                artist_name={album.artistName}
                first_release_date={album.releaseYear}
              />
              {badge && (
                <span className="pointer-events-none absolute left-2 top-2 z-2 rounded bg-black/75 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-200 backdrop-blur">
                  {badge}
                </span>
              )}
            </div>
          );
        })}
      </div>
      <InfiniteLoad
        hasMore={pages.hasNextPage}
        loading={pages.isFetchingNextPage}
        onLoadMore={() => void pages.fetchNextPage()}
      />
    </div>
  );
}

function Compositions({
  composerId,
  focus,
  onClear,
}: {
  composerId: string;
  focus: FocusQuery;
  onClear: () => void;
}) {
  const compositions = useQuery({
    queryKey: ["library", "focus", "compositions", focusQueryKey(focus)],
    queryFn: () => getFocusCompositions(focus),
    placeholderData: keepPreviousData,
  });
  const [open, setOpen] = useState<string | null>(null);
  const data = compositions.data;
  if (compositions.isPending && !data) return null;
  if (compositions.isError && !data) {
    return (
      <LibraryMessage
        action="Try again"
        onAction={() => void compositions.refetch()}
        title="Compositions could not be loaded"
      />
    );
  }
  if (!data?.compositions.length && isFocused(focus)) {
    return <FocusEmpty onClear={onClear} />;
  }
  if (!data?.compositions.length) {
    return (
      <LibraryMessage
        title="No compositions"
        body="No tracks credit this composer yet."
      />
    );
  }
  return (
    <div className="space-y-4">
      <p aria-live="polite" className="text-sm tabular-nums text-zinc-500">
        {data.total.toLocaleString("en-US")}{" "}
        {data.total === 1 ? "composition" : "compositions"} ·{" "}
        {data.trackTotal.toLocaleString("en-US")}{" "}
        {data.trackTotal === 1 ? "track" : "tracks"}
        {data.truncated && " (first 5,000 tracks)"}
      </p>
      <ul className="overflow-hidden rounded-xl border border-white/8">
        {data.compositions.map((composition) => (
          <CompositionRow
            composerId={composerId}
            composition={composition}
            expanded={open === composition.key}
            key={composition.key}
            onToggle={() =>
              setOpen((current) =>
                current === composition.key ? null : composition.key,
              )
            }
          />
        ))}
      </ul>
    </div>
  );
}

function CompositionRow({
  composerId,
  composition,
  expanded,
  onToggle,
}: {
  composerId: string;
  composition: Composition;
  expanded: boolean;
  onToggle: () => void;
}) {
  const playQueue = usePlayQueue();
  const first = composition.recordings[0];
  const others = composition.composers.filter(
    (composer) => composer.id !== composerId,
  );
  return (
    <li className="border-b border-white/6 last:border-0">
      <div className="flex items-center gap-2 px-3 py-2 hover:bg-white/[0.035] sm:px-4">
        <button
          aria-label={`Play ${composition.title}`}
          className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
          disabled={!first}
          onClick={() => first && playQueue(recordingQueue(first), false)}
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
              {composition.title}
            </span>
            <span className="mt-0.5 block truncate text-xs text-zinc-500">
              {compositionSummary(composition)}
              {others.length > 0 &&
                ` · with ${others.map((composer) => composer.name).join(", ")}`}
            </span>
          </span>
          <span className="hidden text-xs tabular-nums text-zinc-500 sm:block">
            {first ? formatDuration(first.durationSeconds) : ""}
          </span>
          <ChevronDown
            className={cn(
              "h-4 w-4 shrink-0 text-zinc-500 transition-transform",
              expanded && "rotate-180",
            )}
          />
        </button>
      </div>
      {expanded && (
        <div className="space-y-3 border-t border-white/6 bg-black/20 px-3 py-3 sm:px-4">
          {composition.recordings.map((recording, index) => (
            <RecordingRow
              key={`${recording.albumId ?? "loose"}-${index}`}
              onPlay={() => playQueue(recordingQueue(recording), false)}
              recording={recording}
            />
          ))}
        </div>
      )}
    </li>
  );
}

function RecordingRow({
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
