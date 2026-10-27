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
import { Input } from "@/components/ui/input";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import { cn } from "@/lib/cn";
import {
  chooseAlbumVersion,
  getAlbumVersionGroups,
  groupAlbumVersions,
  searchLibrary,
  ungroupAlbumVersions,
  type CombinedItem,
  type VersionAlbum,
  type VersionGroup,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  EyeOff,
  Layers,
  Loader2,
  Plus,
  RotateCcw,
  Search,
  Star,
  Unlink,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";
import { getApiErrorMessage } from "./tool-errors";
import {
  EmptyState,
  QualityBadge,
  ToolCard,
  ToolCover,
} from "./tool-primitives";
import { formatCount, formatPlayingTime } from "./tools-model";

const versionsKey = ["library-tools", "versions"] as const;

export function VersionsPanel() {
  const queryClient = useQueryClient();
  const versions = useQuery({
    queryKey: versionsKey,
    queryFn: getAlbumVersionGroups,
  });
  const [groupOpen, setGroupOpen] = useState(false);
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
    await invalidateCatalogRevisionQueries(queryClient);
  };
  const groups = versions.data ?? [];
  const hiding = groups.filter((group) => group.hide_others).length;

  return (
    <div className="space-y-6">
      <ToolCard
        action={
          <Button onClick={() => setGroupOpen(true)} variant="secondary">
            <Plus />
            Group albums
          </Button>
        }
        description="Remasters, deluxe editions and other releases of one album, grouped by title and MusicBrainz release group. The preferred version is the one to play; hide the others to keep one album in your library."
        title={
          <span className="flex items-baseline gap-2">
            Album versions
            <span className="text-sm font-normal tabular-nums text-zinc-500">
              {formatCount(groups.length)}
            </span>
            {hiding > 0 && (
              <span className="text-xs font-normal text-zinc-500">
                · {formatCount(hiding)} showing one version
              </span>
            )}
          </span>
        }
      />
      {versions.isPending ? (
        <div className="flex h-40 items-center justify-center text-zinc-500">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      ) : groups.length === 0 ? (
        <EmptyState icon={<Layers />} title="No album has another version">
          When you add a remaster or edition of an album you have, it appears
          here. Albums with different titles can be grouped by hand.
        </EmptyState>
      ) : (
        groups.map((group) => (
          <VersionGroupCard group={group} key={group.key} onChanged={refresh} />
        ))
      )}
      {groupOpen && (
        <GroupAlbumsDialog
          onClose={() => setGroupOpen(false)}
          onGrouped={refresh}
        />
      )}
    </div>
  );
}

export function VersionGroupCard({
  group,
  onChanged,
  compact = false,
}: {
  group: VersionGroup;
  onChanged: () => Promise<void>;
  compact?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const run = async (work: () => Promise<void>, message: string) => {
    setBusy(true);
    try {
      await work();
      toast.success(message);
      await onChanged();
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not change the versions."));
    } finally {
      setBusy(false);
    }
  };
  const prefer = (album: VersionAlbum) =>
    run(
      () => chooseAlbumVersion(group.key, album.album_id, group.hide_others),
      `${album.edition} is the preferred version`,
    );

  return (
    <article
      className={cn(
        "rounded-xl border border-white/[0.07] bg-white/[0.02]",
        compact ? "p-4" : "p-5",
      )}
    >
      <header className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-white">
            {group.title}
            <span className="font-normal text-zinc-500">
              {" "}
              · {group.artist_name}
            </span>
          </h3>
          <p className="text-xs text-zinc-500">
            {formatCount(group.versions.length)} versions ·{" "}
            {group.manual
              ? "grouped by hand"
              : group.chosen
                ? "preferred version chosen by you"
                : "highest quality preferred automatically"}
          </p>
        </div>
        <div className="flex items-center gap-1">
          <label className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-xs text-zinc-300 hover:bg-white/[0.05]">
            <input
              checked={group.hide_others}
              className="h-3.5 w-3.5 accent-mujin-copper"
              disabled={busy}
              onChange={(event) =>
                void run(
                  () =>
                    chooseAlbumVersion(
                      group.key,
                      group.preferred,
                      event.target.checked,
                    ),
                  event.target.checked
                    ? "Only the preferred version shows in your library"
                    : "Every version shows in your library",
                )
              }
              type="checkbox"
            />
            <EyeOff className="h-3.5 w-3.5 text-zinc-500" />
            Show only the preferred version
          </label>
          {group.chosen && (
            <Button
              disabled={busy}
              onClick={() =>
                void run(
                  () => chooseAlbumVersion(group.key, null),
                  "Back to automatic",
                )
              }
              size="sm"
              title="Prefer the highest quality version and show all"
              variant="ghost"
            >
              <RotateCcw />
              Automatic
            </Button>
          )}
        </div>
      </header>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(168px,1fr))] gap-3">
        {group.versions.map((album) => (
          <div
            className={cn(
              "group relative rounded-lg border p-2.5 transition-colors",
              album.preferred
                ? "border-mujin-copper/45 bg-mujin-copper/[0.06] shadow-[0_0_24px_-12px_#b26bff]"
                : "border-white/[0.06] bg-black/20",
              album.hidden && "opacity-60",
            )}
            key={album.album_id}
          >
            <div className="relative">
              <ToolCover
                alt={album.title}
                className="!h-auto !w-full aspect-square"
                size={160}
                src={album.cover_url}
              />
              {album.preferred && (
                <span className="absolute left-2 top-2 flex items-center gap-1 rounded-full bg-black/75 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-mujin-copper-hover backdrop-blur">
                  <Star className="h-3 w-3 fill-current" />
                  Preferred
                </span>
              )}
              {album.hidden && (
                <span className="absolute left-2 top-2 rounded-full bg-black/75 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-zinc-400">
                  Hidden
                </span>
              )}
            </div>
            <p
              className="mt-2 truncate text-sm font-medium text-zinc-100"
              title={album.title}
            >
              {album.edition}
            </p>
            <p className="truncate text-xs text-zinc-500">
              {[
                album.year,
                `${album.track_count} tracks`,
                formatPlayingTime(album.duration),
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
            <div className="mt-2 flex items-center gap-1.5">
              <QualityBadge quality={album.quality} />
              {album.mixed && (
                <span className="text-[10px] text-zinc-500">mixed</span>
              )}
            </div>
            <div className="mt-2.5 flex items-center gap-1">
              {!album.preferred && (
                <Button
                  className="h-7 flex-1 px-2 text-xs"
                  disabled={busy}
                  onClick={() => void prefer(album)}
                  size="sm"
                  variant="secondary"
                >
                  <Check />
                  Prefer
                </Button>
              )}
              {!album.hidden && (
                <Button
                  asChild
                  className="h-7 flex-1 px-2 text-xs"
                  size="sm"
                  variant="ghost"
                >
                  <Link
                    href={`/album?id=${encodeURIComponent(album.album_id)}`}
                  >
                    Open
                  </Link>
                </Button>
              )}
              <Button
                aria-label={`Take ${album.title} out of this group`}
                className="h-7 w-7 px-0"
                disabled={busy}
                onClick={() =>
                  void run(
                    () => ungroupAlbumVersions([album.album_id]),
                    `${album.title} is no longer grouped`,
                  )
                }
                size="sm"
                title="Not a version of this album"
                variant="ghost"
              >
                <Unlink />
              </Button>
            </div>
          </div>
        ))}
      </div>
    </article>
  );
}

function GroupAlbumsDialog({
  onClose,
  onGrouped,
}: {
  onClose: () => void;
  onGrouped: () => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Map<string, CombinedItem>>(
    new Map(),
  );
  const [busy, setBusy] = useState(false);
  const results = useQuery({
    queryKey: ["library-tools", "album-search", query],
    queryFn: () => searchLibrary(query),
    enabled: query.trim().length >= 2,
  });
  const albums = (results.data ?? []).filter(
    (item) => item.item_type === "album",
  );
  const toggle = (item: CombinedItem) => {
    const next = new Map(selected);
    if (next.has(item.id)) next.delete(item.id);
    else next.set(item.id, item);
    setSelected(next);
  };
  const save = async () => {
    setBusy(true);
    try {
      await groupAlbumVersions([...selected.keys()]);
      toast.success("Grouped as versions of one album");
      await onGrouped();
      onClose();
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not group these albums."));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog onOpenChange={(open) => !open && onClose()} open>
      <DialogContent className="max-w-xl [&>*]:min-w-0">
        <DialogHeader>
          <DialogTitle>Group albums as versions</DialogTitle>
          <DialogDescription>
            For releases detection can't link, such as a translated title.
            Choose two or more albums.
          </DialogDescription>
        </DialogHeader>
        <div className="relative">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-zinc-500" />
          <Input
            aria-label="Search albums"
            autoFocus
            className="pl-9"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search your albums"
            value={query}
          />
        </div>
        {selected.size > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {[...selected.values()].map((item) => (
              <button
                className="rounded-full bg-mujin-copper/15 px-2.5 py-1 text-xs text-mujin-copper-hover"
                key={item.id}
                onClick={() => toggle(item)}
                type="button"
              >
                {item.name} ×
              </button>
            ))}
          </div>
        )}
        <ul className="max-h-72 space-y-1 overflow-y-auto">
          {results.isFetching && (
            <li className="flex justify-center py-4 text-zinc-500">
              <Loader2 className="h-4 w-4 animate-spin" />
            </li>
          )}
          {albums.map((item) => (
            <li key={item.id}>
              <button
                aria-pressed={selected.has(item.id)}
                className={cn(
                  "flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors",
                  selected.has(item.id)
                    ? "bg-white/[0.07]"
                    : "hover:bg-white/[0.04]",
                )}
                onClick={() => toggle(item)}
                type="button"
              >
                <ToolCover
                  alt={item.name}
                  size={36}
                  src={item.album_object?.cover_url ?? ""}
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm text-zinc-100">
                    {item.name}
                  </span>
                  {item.description && (
                    <span className="block truncate text-xs text-zinc-500">
                      {item.description}
                    </span>
                  )}
                </span>
                {selected.has(item.id) && (
                  <Check className="h-4 w-4 text-mujin-copper" />
                )}
              </button>
            </li>
          ))}
        </ul>
        <DialogFooter>
          <Button onClick={onClose} variant="ghost">
            Cancel
          </Button>
          <Button
            disabled={busy || selected.size < 2}
            onClick={() => void save()}
          >
            {busy ? <Loader2 className="animate-spin" /> : <Layers />}
            Group {selected.size > 1 ? selected.size : ""} albums
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
