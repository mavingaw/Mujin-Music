"use client";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  MATCH_REASON_LABELS,
  confidenceLabel,
  deleteImportJob,
  importErrorMessage,
  importMatchRate,
  rematchImportJob,
  saveImportPlaylist,
  updateImportItems,
  type ImportItem,
  type ImportItemAction,
  type ImportJob,
  type ImportSongBrief,
} from "@parson/music-sdk";
import {
  ArrowRight,
  Check,
  ChevronDown,
  ExternalLink,
  ListMusic,
  Loader2,
  RefreshCw,
  Search,
  SkipForward,
  Trash2,
  Undo2,
} from "lucide-react";
import Link from "next/link";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
  REVIEW_FILTERS,
  SERVICE_ACCENTS,
  SERVICE_SHORT_NAMES,
  acceptAllSuggestions,
  defaultFilter,
  filterCounts,
  formatTrackDuration,
  itemMatchesFilter,
  percent,
  skipAllUnmatched,
  type ReviewFilter,
} from "./import-review-state";
import SongPickerDialog from "./song-picker-dialog";

const PAGE_SIZE = 200;

function ConfidencePill({ value }: { value: number }) {
  const band = confidenceLabel(value);
  const tone = {
    high: "border-emerald-400/30 bg-emerald-400/10 text-emerald-200",
    medium: "border-amber-400/30 bg-amber-400/10 text-amber-200",
    low: "border-rose-400/30 bg-rose-400/10 text-rose-200",
  }[band];
  return (
    <span
      className={`inline-flex shrink-0 items-center rounded-full border px-1.5 py-0.5 text-[10px] font-semibold tabular-nums ${tone}`}
      title={`${band} confidence`}
    >
      {percent(value)}
    </span>
  );
}

function Cover({ song }: { song?: ImportSongBrief }) {
  const src = song ? getLibraryImageUrl(song.cover_url, getBaseURL) : null;
  return src ? (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      alt=""
      className="h-10 w-10 shrink-0 rounded object-cover"
      loading="lazy"
      src={src}
    />
  ) : (
    <div className="grid h-10 w-10 shrink-0 place-items-center rounded bg-white/5 text-zinc-600">
      <ListMusic className="h-4 w-4" />
    </div>
  );
}

function StatusLabel({ item }: { item: ImportItem }) {
  const labels: Record<ImportItem["status"], [string, string]> = {
    matched: ["Matched", "text-emerald-300"],
    confirmed: ["Confirmed", "text-emerald-300"],
    review: ["Check", "text-amber-300"],
    unmatched: ["Not in library", "text-rose-300"],
    skipped: ["Skipped", "text-zinc-500"],
  };
  const [label, tone] = labels[item.status];
  return <span className={`text-[11px] font-medium ${tone}`}>{label}</span>;
}

function ItemRow({
  item,
  songs,
  busy,
  onAction,
  onSearch,
}: {
  item: ImportItem;
  songs: Record<string, ImportSongBrief>;
  busy: boolean;
  onAction: (action: ImportItemAction) => void;
  onSearch: (item: ImportItem) => void;
}) {
  const song = item.song_id ? songs[item.song_id] : undefined;
  const alternatives = item.candidates.filter(
    (candidate) =>
      candidate.song_id !== item.song_id && songs[candidate.song_id],
  );
  const skipped = item.status === "skipped";
  const sourceDuration = formatTrackDuration(item.source.duration_ms);
  return (
    <li
      className={`grid gap-3 border-b border-white/5 px-3 py-3 md:grid-cols-[2rem_minmax(0,1fr)_1.25rem_minmax(0,1.1fr)_auto] md:items-center ${
        skipped ? "opacity-55" : ""
      }`}
    >
      <span className="hidden text-right text-xs tabular-nums text-zinc-600 md:block">
        {item.position + 1}
      </span>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="truncate text-sm text-zinc-100">
            {item.source.title}
          </span>
          {item.source.external_url && (
            <a
              aria-label="Open on the original service"
              className="shrink-0 text-zinc-600 hover:text-zinc-300"
              href={item.source.external_url}
              rel="noreferrer"
              target="_blank"
            >
              <ExternalLink className="h-3 w-3" />
            </a>
          )}
        </div>
        <p className="truncate text-xs text-zinc-500">
          {[item.source.artists.join(", "), item.source.album, sourceDuration]
            .filter(Boolean)
            .join(" · ") || "Title only"}
        </p>
      </div>
      <ArrowRight className="hidden h-4 w-4 text-zinc-700 md:block" />
      <div className="flex min-w-0 items-center gap-3">
        {song ? (
          <>
            <Cover song={song} />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-2">
                <span className="truncate text-sm text-zinc-100">
                  {song.name}
                </span>
                <ConfidencePill value={item.confidence} />
              </div>
              <p className="truncate text-xs text-zinc-500">
                {[
                  song.artist,
                  song.album,
                  formatTrackDuration(song.duration * 1000),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
              <p className="mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px] text-zinc-600">
                <StatusLabel item={item} />
                {item.reasons.slice(0, 4).map((reason) => (
                  <span key={reason}>{MATCH_REASON_LABELS[reason]}</span>
                ))}
              </p>
            </div>
          </>
        ) : (
          <div className="min-w-0">
            <StatusLabel item={item} />
            <p className="text-xs text-zinc-600">
              {alternatives.length
                ? `${alternatives.length} weak suggestion${alternatives.length === 1 ? "" : "s"}`
                : "Nothing similar in your library"}
            </p>
          </div>
        )}
      </div>
      <div className="flex items-center gap-1 md:justify-end">
        {item.status === "review" && item.song_id && (
          <Button
            disabled={busy}
            onClick={() =>
              onAction({ action: "accept", position: item.position })
            }
            size="sm"
            variant="outline"
          >
            <Check /> Accept
          </Button>
        )}
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={`Change match for ${item.source.title}`}
              disabled={busy}
              size="sm"
              variant="ghost"
            >
              Change <ChevronDown />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-72">
            {alternatives.length > 0 && (
              <>
                <DropdownMenuLabel>Suggestions</DropdownMenuLabel>
                {alternatives.map((candidate) => {
                  const option = songs[candidate.song_id]!;
                  return (
                    <DropdownMenuItem
                      key={candidate.song_id}
                      onSelect={() =>
                        onAction({
                          action: "choose",
                          position: item.position,
                          song_id: candidate.song_id,
                        })
                      }
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate">{option.name}</span>
                        <span className="block truncate text-xs text-zinc-500">
                          {option.artist} · {option.album}
                        </span>
                      </span>
                      <ConfidencePill value={candidate.confidence} />
                    </DropdownMenuItem>
                  );
                })}
                <DropdownMenuSeparator />
              </>
            )}
            <DropdownMenuItem onSelect={() => onSearch(item)}>
              <Search className="h-4 w-4" /> Search your library…
            </DropdownMenuItem>
            {skipped ? (
              <DropdownMenuItem
                onSelect={() =>
                  onAction({ action: "reset", position: item.position })
                }
              >
                <Undo2 className="h-4 w-4" /> Include again
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem
                onSelect={() =>
                  onAction({ action: "skip", position: item.position })
                }
              >
                <SkipForward className="h-4 w-4" /> Leave out
              </DropdownMenuItem>
            )}
            {item.status === "confirmed" && (
              <DropdownMenuItem
                onSelect={() =>
                  onAction({ action: "reset", position: item.position })
                }
              >
                <RefreshCw className="h-4 w-4" /> Match again
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </li>
  );
}

export default function ImportReview({
  job,
  onJobChange,
  onDeleted,
}: {
  job: ImportJob;
  onJobChange: (job: ImportJob) => void;
  onDeleted: () => void;
}) {
  const [filter, setFilter] = useState<ReviewFilter>(() =>
    defaultFilter(job.items),
  );
  const [busy, setBusy] = useState(false);
  const [limit, setLimit] = useState(PAGE_SIZE);
  const [name, setName] = useState(job.name);
  const [picking, setPicking] = useState<ImportItem | null>(null);
  const counts = useMemo(() => filterCounts(job.items), [job.items]);
  const visible = useMemo(
    () => job.items.filter((item) => itemMatchesFilter(item, filter)),
    [filter, job.items],
  );
  const rate = importMatchRate(job);
  const bulkAccept = acceptAllSuggestions(job.items);
  const bulkSkip = skipAllUnmatched(job.items);

  async function run<T>(work: () => Promise<T>, failure: string) {
    if (busy) return undefined;
    setBusy(true);
    try {
      return await work();
    } catch (error) {
      toast.error(importErrorMessage(error, failure));
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  const apply = (actions: ImportItemAction[]) =>
    void run(async () => {
      onJobChange(await updateImportItems(job.id, actions));
    }, "The change was not saved.");

  const save = () =>
    void run(async () => {
      const result = await saveImportPlaylist(job.id, name);
      toast.success(
        result.created
          ? `Created “${name || job.name}” with ${result.added} tracks.`
          : result.added
            ? `Added ${result.added} newly matched tracks to the playlist.`
            : "The playlist is already up to date.",
      );
      onJobChange({ ...job, playlist_id: result.playlist_id });
    }, "The playlist was not saved.");

  const rematch = () =>
    void run(async () => {
      const updated = await rematchImportJob(job.id);
      onJobChange(updated);
      toast.success("Matched again against your current library.");
    }, "Matching again failed.");

  const remove = () =>
    void run(async () => {
      await deleteImportJob(job.id);
      onDeleted();
    }, "The import was not deleted.");

  return (
    <section aria-label={`Review ${job.name}`} className="grid gap-5">
      <header className="grid gap-4 md:flex md:items-end">
        <div className="min-w-0 md:flex-1">
          <p className="flex items-center gap-2 text-xs font-medium uppercase tracking-wider text-zinc-500">
            <span
              aria-hidden
              className={`h-2 w-2 rounded-full ${SERVICE_ACCENTS[job.service]}`}
            />
            {SERVICE_SHORT_NAMES[job.service]} import
            {job.source_url && (
              <a
                className="inline-flex items-center gap-1 normal-case tracking-normal text-zinc-500 hover:text-zinc-300"
                href={job.source_url}
                rel="noreferrer"
                target="_blank"
              >
                Source <ExternalLink className="h-3 w-3" />
              </a>
            )}
          </p>
          <h2 className="mt-1 break-words text-2xl font-semibold text-white md:truncate">
            {job.name}
          </h2>
          <p className="mt-1 text-sm text-zinc-500">
            {job.matched} of {job.total - job.skipped} tracks will play from
            your library ({percent(rate)}). Nothing is streamed from{" "}
            {job.service === "file"
              ? "anywhere else"
              : SERVICE_SHORT_NAMES[job.service]}
            .
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button disabled={busy} onClick={rematch} size="sm" variant="ghost">
            <RefreshCw /> Match again
          </Button>
          <Button disabled={busy} onClick={remove} size="sm" variant="ghost">
            <Trash2 /> Delete import
          </Button>
        </div>
      </header>

      <div
        aria-label="Match summary"
        className="flex h-2 overflow-hidden rounded-full bg-white/5"
        role="img"
      >
        {(
          [
            ["matched", "bg-emerald-400"],
            ["review", "bg-amber-400"],
            ["unmatched", "bg-rose-500/80"],
            ["skipped", "bg-zinc-600"],
          ] as const
        ).map(([key, tone]) =>
          counts[key] ? (
            <span
              className={tone}
              key={key}
              style={{
                width: `${(counts[key] / Math.max(1, job.total)) * 100}%`,
              }}
            />
          ) : null,
        )}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div
          aria-label="Filter tracks"
          className="flex max-w-full gap-1 overflow-x-auto rounded-lg bg-white/4 p-1"
          role="tablist"
        >
          {REVIEW_FILTERS.map(({ id, label }) => (
            <button
              aria-selected={filter === id}
              className={`shrink-0 rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                filter === id
                  ? "bg-white/10 text-white"
                  : "text-zinc-500 hover:text-zinc-200"
              }`}
              key={id}
              onClick={() => {
                setFilter(id);
                setLimit(PAGE_SIZE);
              }}
              role="tab"
              type="button"
            >
              {label}
              <span className="ml-1.5 tabular-nums text-zinc-500">
                {counts[id]}
              </span>
            </button>
          ))}
        </div>
        <div className="flex flex-wrap gap-2 md:ml-auto">
          {bulkAccept.length > 0 && (
            <Button
              disabled={busy}
              onClick={() => apply(bulkAccept)}
              size="sm"
              variant="outline"
            >
              <Check /> Accept {bulkAccept.length}{" "}
              {bulkAccept.length === 1 ? "suggestion" : "suggestions"}
            </Button>
          )}
          {bulkSkip.length > 0 && (
            <Button
              disabled={busy}
              onClick={() => apply(bulkSkip)}
              size="sm"
              variant="ghost"
            >
              <SkipForward /> Leave out {bulkSkip.length} not found
            </Button>
          )}
        </div>
      </div>

      <ul className="rounded-xl border border-white/8 bg-white/2">
        {visible.length === 0 ? (
          <li className="px-4 py-10 text-center text-sm text-zinc-500">
            {filter === "review"
              ? "Nothing left to review."
              : "No tracks in this view."}
          </li>
        ) : (
          visible
            .slice(0, limit)
            .map((item) => (
              <ItemRow
                busy={busy}
                item={item}
                key={item.position}
                onAction={(action) => apply([action])}
                onSearch={setPicking}
                songs={job.songs}
              />
            ))
        )}
        {visible.length > limit && (
          <li className="p-3 text-center">
            <Button
              onClick={() => setLimit((value) => value + PAGE_SIZE)}
              size="sm"
              variant="ghost"
            >
              Show {Math.min(PAGE_SIZE, visible.length - limit)} more
            </Button>
          </li>
        )}
      </ul>

      <footer className="z-10 flex flex-wrap items-center gap-3 rounded-xl border border-white/10 bg-[#0b0b0b]/95 p-3 shadow-2xl shadow-black/60 backdrop-blur md:sticky md:bottom-28">
        <Input
          aria-label="Playlist name"
          className="min-w-48 flex-1"
          maxLength={200}
          onChange={(event) => setName(event.target.value)}
          value={name}
        />
        {job.playlist_id && (
          <Button asChild variant="ghost">
            <Link href={`/playlist?id=${job.playlist_id}`}>Open playlist</Link>
          </Button>
        )}
        <Button disabled={busy || job.matched === 0} onClick={save}>
          {busy ? <Loader2 className="animate-spin" /> : <ListMusic />}
          {job.playlist_id ? "Update playlist" : "Save as playlist"}
          <span className="tabular-nums opacity-70">({job.matched})</span>
        </Button>
      </footer>

      {picking && (
        <SongPickerDialog
          key={picking.position}
          onOpenChange={(open) => {
            if (!open) setPicking(null);
          }}
          onPick={(songId) =>
            apply([
              { action: "choose", position: picking.position, song_id: songId },
            ])
          }
          open
          source={picking.source}
        />
      )}
    </section>
  );
}
