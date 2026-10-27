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
  albumizeTracks,
  deleteUserAlbum,
  getAlbumizer,
  searchLibrary,
  type AlbumSuggestion,
  type UserAlbum,
  type UserAlbumType,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
  ArrowUp,
  Disc3,
  FolderOpen,
  Loader2,
  Pencil,
  Plus,
  Search,
  Undo2,
  WandSparkles,
  X,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";
import { getApiErrorMessage } from "./tool-errors";
import { EmptyState, ToolCard, ToolCover } from "./tool-primitives";
import {
  albumTypeLabels,
  formatCount,
  formatTrackTime,
  moveItem,
} from "./tools-model";

const albumizerKey = ["library-tools", "albumizer"] as const;

type DraftTrack = {
  id: string;
  title: string;
  artist: string;
  cover_url: string;
  duration: number;
};

type Draft = {
  id?: string;
  title: string;
  artist: string;
  year: string;
  type: UserAlbumType;
  coverSongId?: string;
  tracks: DraftTrack[];
};

function draftFromSuggestion(suggestion: AlbumSuggestion): Draft {
  return {
    title: suggestion.title,
    artist: suggestion.artist,
    year: "",
    type: suggestion.tracks.length <= 3 ? "ep" : "album",
    coverSongId: suggestion.tracks.find((track) => track.cover_url)?.song_id,
    tracks: suggestion.tracks.map((track) => ({
      id: track.song_id,
      title: track.title,
      artist: track.artist,
      cover_url: track.cover_url,
      duration: track.duration,
    })),
  };
}

function draftFromAlbum(album: UserAlbum): Draft {
  return {
    id: album.id,
    title: album.title,
    artist: album.artist_name,
    year: album.year,
    type: album.primary_type,
    tracks: album.tracks.map((track) => ({
      id: track.id,
      title: track.title,
      artist: track.artist,
      cover_url: track.cover_url,
      duration: track.duration,
    })),
  };
}

export function AlbumizerPanel() {
  const queryClient = useQueryClient();
  const albumizer = useQuery({ queryKey: albumizerKey, queryFn: getAlbumizer });
  const [draft, setDraft] = useState<Draft | null>(null);
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
    await invalidateCatalogRevisionQueries(queryClient);
  };
  const suggestions = albumizer.data?.suggestions ?? [];
  const made = albumizer.data?.albums ?? [];
  const looseCount = suggestions.reduce(
    (sum, suggestion) => sum + suggestion.tracks.length,
    0,
  );

  return (
    <div className="space-y-6">
      <ToolCard
        action={
          <Button
            onClick={() =>
              setDraft({
                title: "",
                artist: "",
                year: "",
                type: "album",
                tracks: [],
              })
            }
            variant="secondary"
          >
            <Plus />
            New album
          </Button>
        }
        description="Group loose tracks into an album and give it a proper identity: title, artist, year, type and cover. Your files and tags stay as they are; undo returns the tracks to where the scan put them."
        title={
          <span className="flex items-baseline gap-2">
            Albumizer
            {looseCount > 0 && (
              <span className="text-sm font-normal tabular-nums text-zinc-500">
                {formatCount(looseCount)} loose tracks
              </span>
            )}
          </span>
        }
      />

      {albumizer.isPending ? (
        <div className="flex h-40 items-center justify-center text-zinc-500">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      ) : (
        <>
          {made.length > 0 && (
            <section>
              <h2 className="mb-3 text-[15px] font-semibold text-white">
                Albums you made
              </h2>
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {made.map((album) => (
                  <div
                    className="flex items-center gap-3 rounded-xl border border-white/[0.07] bg-white/[0.02] p-3"
                    key={album.id}
                  >
                    <ToolCover
                      alt={album.title}
                      size={56}
                      src={album.cover_url}
                    />
                    <div className="min-w-0 flex-1">
                      <Link
                        className="block truncate text-sm font-medium text-zinc-100 hover:underline"
                        href={`/album?id=${encodeURIComponent(album.id)}`}
                      >
                        {album.title}
                      </Link>
                      <p className="truncate text-xs text-zinc-500">
                        {album.artist_name} ·{" "}
                        {albumTypeLabels[album.primary_type] ?? "Album"}
                        {album.year ? ` · ${album.year}` : ""} ·{" "}
                        {album.tracks.length} tracks
                      </p>
                    </div>
                    <Button
                      aria-label={`Edit ${album.title}`}
                      onClick={() => setDraft(draftFromAlbum(album))}
                      size="icon"
                      variant="ghost"
                    >
                      <Pencil />
                    </Button>
                    <Button
                      aria-label={`Undo ${album.title}`}
                      onClick={() =>
                        void deleteUserAlbum(album.id)
                          .then(refresh)
                          .then(() =>
                            toast.success(
                              "Tracks are back where the scan put them",
                            ),
                          )
                          .catch((error: unknown) =>
                            toast(
                              getApiErrorMessage(
                                error,
                                "Could not undo this album.",
                              ),
                            ),
                          )
                      }
                      size="icon"
                      title="Undo this album"
                      variant="ghost"
                    >
                      <Undo2 />
                    </Button>
                  </div>
                ))}
              </div>
            </section>
          )}

          <section>
            <h2 className="mb-1 text-[15px] font-semibold text-white">
              Loose tracks by folder
            </h2>
            <p className="mb-3 text-sm text-zinc-500">
              Each of these tracks sits alone on an album of one. Tracks that
              share a folder usually belong together.
            </p>
            {suggestions.length === 0 ? (
              <EmptyState icon={<Disc3 />} title="No loose tracks">
                Every track belongs to an album with others.
              </EmptyState>
            ) : (
              <div className="grid gap-3 lg:grid-cols-2">
                {suggestions.slice(0, 60).map((suggestion) => (
                  <article
                    className="rounded-xl border border-white/[0.07] bg-white/[0.02] p-4"
                    key={suggestion.folder}
                  >
                    <header className="flex items-start gap-3">
                      <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-white/[0.04] text-zinc-400">
                        <FolderOpen className="h-5 w-5" />
                      </span>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-medium text-zinc-100">
                          {suggestion.title}
                        </p>
                        <p
                          className="truncate font-mono text-[11px] text-zinc-600"
                          title={suggestion.folder}
                        >
                          {suggestion.folder}
                        </p>
                      </div>
                      <Button
                        onClick={() =>
                          setDraft(draftFromSuggestion(suggestion))
                        }
                        size="sm"
                      >
                        <WandSparkles />
                        Make album
                      </Button>
                    </header>
                    <ol className="mt-3 space-y-1">
                      {suggestion.tracks.slice(0, 6).map((track, index) => (
                        <li
                          className="flex items-center gap-3 text-xs text-zinc-400"
                          key={track.song_id}
                        >
                          <span className="w-4 text-right tabular-nums text-zinc-600">
                            {index + 1}
                          </span>
                          <span className="min-w-0 flex-1 truncate text-zinc-300">
                            {track.title}
                          </span>
                          <span className="truncate text-zinc-500">
                            {track.artist}
                          </span>
                          <span className="tabular-nums text-zinc-600">
                            {formatTrackTime(track.duration)}
                          </span>
                        </li>
                      ))}
                      {suggestion.tracks.length > 6 && (
                        <li className="pl-7 text-xs text-zinc-600">
                          and {suggestion.tracks.length - 6} more
                        </li>
                      )}
                    </ol>
                  </article>
                ))}
              </div>
            )}
          </section>
        </>
      )}

      {draft && (
        <AlbumEditor
          draft={draft}
          onClose={() => setDraft(null)}
          onSaved={refresh}
        />
      )}
    </div>
  );
}

function AlbumEditor({
  draft: initial,
  onClose,
  onSaved,
}: {
  draft: Draft;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [draft, setDraft] = useState<Draft>(initial);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const songs = useQuery({
    queryKey: ["library-tools", "song-search", query],
    queryFn: () => searchLibrary(query),
    enabled: query.trim().length >= 2,
  });
  const results = (songs.data ?? []).filter(
    (item) =>
      item.item_type === "song" &&
      !draft.tracks.some((track) => track.id === item.id),
  );
  const covers = draft.tracks.filter(
    (track, index, all) =>
      track.cover_url &&
      all.findIndex((other) => other.cover_url === track.cover_url) === index,
  );
  const valid =
    draft.title.trim() &&
    draft.artist.trim() &&
    draft.tracks.length > 0 &&
    (!draft.year || /^\d{4}$/.test(draft.year));

  const save = async () => {
    setBusy(true);
    try {
      await albumizeTracks({
        id: draft.id,
        title: draft.title.trim(),
        artist_name: draft.artist.trim(),
        year: draft.year,
        primary_type: draft.type,
        cover_song_id: draft.coverSongId,
        song_ids: draft.tracks.map((track) => track.id),
      });
      toast.success(`${draft.title.trim()} is in your library`);
      await onSaved();
      onClose();
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not save the album."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog onOpenChange={(open) => !open && onClose()} open>
      <DialogContent className="max-w-2xl [&>*]:min-w-0">
        <DialogHeader>
          <DialogTitle>{draft.id ? "Edit album" : "Make an album"}</DialogTitle>
          <DialogDescription>
            The album exists in Parson only; the files and their tags are not
            changed.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="grid gap-1.5 text-xs text-zinc-400 sm:col-span-2">
            Title
            <Input
              autoFocus
              onChange={(event) =>
                setDraft({ ...draft, title: event.target.value })
              }
              value={draft.title}
            />
          </label>
          <label className="grid gap-1.5 text-xs text-zinc-400">
            Album artist
            <Input
              onChange={(event) =>
                setDraft({ ...draft, artist: event.target.value })
              }
              value={draft.artist}
            />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="grid gap-1.5 text-xs text-zinc-400">
              Year
              <Input
                inputMode="numeric"
                maxLength={4}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    year: event.target.value.replace(/\D/g, ""),
                  })
                }
                placeholder="2024"
                value={draft.year}
              />
            </label>
            <label className="grid gap-1.5 text-xs text-zinc-400">
              Type
              <select
                className="h-10 rounded-md border border-zinc-800 bg-zinc-950 px-2 text-sm text-zinc-100"
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    type: event.target.value as UserAlbumType,
                  })
                }
                value={draft.type}
              >
                {Object.entries(albumTypeLabels).map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </div>

        {covers.length > 1 && (
          <div>
            <p className="mb-2 text-xs text-zinc-400">Cover</p>
            <div className="flex flex-wrap gap-2">
              {covers.map((track) => (
                <button
                  aria-label={`Use the cover of ${track.title}`}
                  aria-pressed={draft.coverSongId === track.id}
                  className={cn(
                    "rounded-md ring-2 ring-offset-2 ring-offset-[#0b0b0b] transition",
                    draft.coverSongId === track.id
                      ? "ring-[#b26bff]"
                      : "ring-transparent",
                  )}
                  key={track.id}
                  onClick={() => setDraft({ ...draft, coverSongId: track.id })}
                  type="button"
                >
                  <ToolCover alt="" size={52} src={track.cover_url} />
                </button>
              ))}
            </div>
          </div>
        )}

        <div>
          <p className="mb-2 text-xs text-zinc-400">
            Tracks in album order · {draft.tracks.length}
          </p>
          <ol className="max-h-64 space-y-1 overflow-y-auto rounded-lg border border-white/[0.06] p-1.5">
            {draft.tracks.length === 0 && (
              <li className="px-2 py-3 text-center text-xs text-zinc-500">
                Search below to add tracks.
              </li>
            )}
            {draft.tracks.map((track, index) => (
              <li
                className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-white/[0.03]"
                key={track.id}
              >
                <span className="w-5 text-right text-xs tabular-nums text-zinc-600">
                  {index + 1}
                </span>
                <span className="min-w-0 flex-1 truncate text-sm text-zinc-200">
                  {track.title}
                  <span className="text-zinc-500"> · {track.artist}</span>
                </span>
                <span className="text-xs tabular-nums text-zinc-600">
                  {formatTrackTime(track.duration)}
                </span>
                <button
                  aria-label={`Move ${track.title} up`}
                  className="flex h-7 w-7 items-center justify-center rounded text-zinc-500 hover:bg-white/8 hover:text-white disabled:opacity-30"
                  disabled={index === 0}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      tracks: moveItem(draft.tracks, index, index - 1),
                    })
                  }
                  type="button"
                >
                  <ArrowUp className="h-3.5 w-3.5" />
                </button>
                <button
                  aria-label={`Move ${track.title} down`}
                  className="flex h-7 w-7 items-center justify-center rounded text-zinc-500 hover:bg-white/8 hover:text-white disabled:opacity-30"
                  disabled={index === draft.tracks.length - 1}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      tracks: moveItem(draft.tracks, index, index + 1),
                    })
                  }
                  type="button"
                >
                  <ArrowDown className="h-3.5 w-3.5" />
                </button>
                <button
                  aria-label={`Remove ${track.title}`}
                  className="flex h-7 w-7 items-center justify-center rounded text-zinc-500 hover:bg-white/8 hover:text-white"
                  onClick={() =>
                    setDraft({
                      ...draft,
                      tracks: draft.tracks.filter(
                        (item) => item.id !== track.id,
                      ),
                    })
                  }
                  type="button"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </li>
            ))}
          </ol>
          <div className="relative mt-2">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-zinc-500" />
            <Input
              aria-label="Add tracks"
              className="h-9 pl-9"
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Add tracks from your library"
              value={query}
            />
          </div>
          {results.length > 0 && (
            <ul className="mt-1 max-h-40 overflow-y-auto rounded-lg border border-white/[0.06] p-1">
              {results.slice(0, 20).map((item) => (
                <li key={item.id}>
                  <button
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-zinc-300 hover:bg-white/[0.05]"
                    onClick={() =>
                      setDraft({
                        ...draft,
                        tracks: [
                          ...draft.tracks,
                          {
                            id: item.id,
                            title: item.name,
                            artist:
                              item.song_object?.artist ??
                              item.description ??
                              "",
                            cover_url: item.album_object?.cover_url ?? "",
                            duration: item.song_object?.duration ?? 0,
                          },
                        ],
                      })
                    }
                    type="button"
                  >
                    <Plus className="h-3.5 w-3.5 text-zinc-500" />
                    <span className="min-w-0 flex-1 truncate">{item.name}</span>
                    {item.description && (
                      <span className="truncate text-xs text-zinc-500">
                        {item.description}
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
        <DialogFooter>
          <Button onClick={onClose} variant="ghost">
            Cancel
          </Button>
          <Button disabled={busy || !valid} onClick={() => void save()}>
            {busy ? <Loader2 className="animate-spin" /> : <Disc3 />}
            {draft.id ? "Save album" : "Make album"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
