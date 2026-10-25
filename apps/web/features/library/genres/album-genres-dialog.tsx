"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { genreSuggestions } from "@parson/music-sdk";
import type { GenreRef } from "@parson/music-sdk/types";
import { Loader2, Plus, X } from "lucide-react";
import { useState } from "react";
import { useGenreTree, useSaveAlbumGenres } from "./use-genres";

/**
 * Administrator: choose an album's genres by hand. Names are matched to the
 * canonical genre tree ("bop" becomes Bebop); unknown names are kept as
 * they are. The choice replaces tag and online genres until it is reset.
 */
export function AlbumGenresDialog({
  albumId,
  albumName,
  genres,
  edited,
  open,
  onOpenChange,
}: {
  albumId: string;
  albumName: string;
  genres: readonly GenreRef[];
  edited: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const tree = useGenreTree(open);
  const save = useSaveAlbumGenres(albumId);
  const [chosen, setChosen] = useState<string[] | null>(null);
  const [query, setQuery] = useState("");
  const current = chosen ?? genres.map((genre) => genre.name);
  const suggestions = genreSuggestions(tree.data, query, current);
  const add = (name: string) => {
    const trimmed = name.trim();
    if (!trimmed || current.length >= 12) return;
    if (
      current.some(
        (existing) => existing.toLowerCase() === trimmed.toLowerCase(),
      )
    )
      return;
    setChosen([...current, trimmed]);
    setQuery("");
  };
  const close = (next: boolean) => {
    if (!next) {
      setChosen(null);
      setQuery("");
    }
    onOpenChange(next);
  };

  return (
    <Dialog onOpenChange={close} open={open}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Genres</DialogTitle>
          <DialogDescription>
            Choose the genres {albumName} shows and is listed under. They
            replace the genres from file tags and MusicBrainz.
          </DialogDescription>
        </DialogHeader>
        <ul
          aria-label="Chosen genres"
          className="flex min-h-8 flex-wrap gap-1.5"
        >
          {current.map((name) => (
            <li key={name}>
              <span className="inline-flex items-center gap-1 rounded-full bg-white/10 py-0.5 pl-2.5 pr-1 text-sm text-zinc-100">
                {name}
                <button
                  aria-label={`Remove ${name}`}
                  className="rounded-full p-0.5 text-zinc-400 hover:bg-white/10 hover:text-white"
                  onClick={() =>
                    setChosen(current.filter((existing) => existing !== name))
                  }
                  type="button"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </span>
            </li>
          ))}
          {current.length === 0 && (
            <li className="text-sm text-zinc-500">No genres</li>
          )}
        </ul>
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            add(suggestions[0] && query.trim() ? suggestions[0] : query);
          }}
        >
          <Input
            aria-label="Add a genre"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Add a genre, e.g. Hard Bop"
            value={query}
          />
          <button
            aria-label="Add genre"
            className="flex h-9 shrink-0 items-center gap-1 rounded-md border border-white/15 px-3 text-sm text-zinc-200 hover:bg-white/8 disabled:opacity-50"
            disabled={!query.trim()}
            type="submit"
          >
            <Plus className="h-4 w-4" /> Add
          </button>
        </form>
        {suggestions.length > 0 && (
          <ul aria-label="Suggested genres" className="flex flex-wrap gap-1.5">
            {suggestions.map((name) => (
              <li key={name}>
                <button
                  className="rounded-full border border-white/10 px-2.5 py-0.5 text-xs text-zinc-300 hover:border-white/25 hover:text-white"
                  onClick={() => add(name)}
                  type="button"
                >
                  {name}
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center justify-between gap-3 pt-2">
          {edited ? (
            <button
              className="text-sm text-zinc-400 hover:text-white disabled:opacity-50"
              disabled={save.isPending}
              onClick={() =>
                save.mutate(null, { onSuccess: () => close(false) })
              }
              type="button"
            >
              Use genres from tags and MusicBrainz
            </button>
          ) : (
            <span />
          )}
          <button
            className="flex h-9 items-center gap-2 rounded-md bg-white px-4 text-sm font-semibold text-black hover:bg-zinc-200 disabled:opacity-50"
            disabled={save.isPending || chosen === null}
            onClick={() =>
              save.mutate(current, { onSuccess: () => close(false) })
            }
            type="button"
          >
            {save.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
            Save
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
