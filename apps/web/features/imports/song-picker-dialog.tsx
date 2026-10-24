"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { searchLibrary, type ImportSourceTrack } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Loader2, Music2 } from "lucide-react";
import { useEffect, useState } from "react";
import { formatTrackDuration } from "./import-review-state";

function initialQuery(source: ImportSourceTrack | null) {
  if (!source) return "";
  return [source.title, source.artists[0]].filter(Boolean).join(" ");
}

/**
 * Lets a person pick the right library song for an imported track. Mount it
 * with a `key` per track so the search starts from that track's title.
 */
export default function SongPickerDialog({
  source,
  open,
  onOpenChange,
  onPick,
}: {
  source: ImportSourceTrack | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (songId: string) => void;
}) {
  const [query, setQuery] = useState(() => initialQuery(source));
  const [debounced, setDebounced] = useState(query);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 200);
    return () => clearTimeout(timer);
  }, [query]);
  const results = useQuery({
    queryKey: ["import-song-search", debounced],
    queryFn: () => searchLibrary(debounced),
    enabled: open && debounced.length > 1,
    staleTime: 30_000,
  });
  const songs = (results.data ?? [])
    .filter((item) => item.item_type === "song")
    .slice(0, 30);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Choose from your library</DialogTitle>
          {source && (
            <DialogDescription>
              For “{source.title}”
              {source.artists.length ? ` by ${source.artists.join(", ")}` : ""}
              {source.duration_ms
                ? ` · ${formatTrackDuration(source.duration_ms)}`
                : ""}
            </DialogDescription>
          )}
        </DialogHeader>
        <Input
          aria-label="Search your library"
          autoFocus
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search songs"
          value={query}
        />
        <div className="max-h-[50dvh] min-h-40 overflow-y-auto">
          {results.isFetching && !songs.length ? (
            <div className="flex items-center gap-2 p-3 text-sm text-zinc-500">
              <Loader2 className="h-4 w-4 animate-spin" /> Searching…
            </div>
          ) : songs.length ? (
            <ul className="grid gap-0.5">
              {songs.map((item) => (
                <li key={item.id}>
                  <button
                    className="flex w-full items-center gap-3 rounded-md px-2 py-2 text-left hover:bg-white/6 focus-visible:bg-white/8 focus-visible:outline-none"
                    onClick={() => {
                      onPick(item.id);
                      onOpenChange(false);
                    }}
                    type="button"
                  >
                    <Music2 className="h-4 w-4 shrink-0 text-zinc-600" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm text-zinc-100">
                        {item.name}
                      </span>
                      <span className="block truncate text-xs text-zinc-500">
                        {[
                          item.song_object?.artist ?? item.artist_object?.name,
                          item.album_object?.name,
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>
                    <span className="shrink-0 text-xs tabular-nums text-zinc-600">
                      {item.song_object?.duration
                        ? formatTrackDuration(item.song_object.duration * 1000)
                        : ""}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="p-3 text-sm text-zinc-500">
              {debounced.length > 1
                ? "No songs found. This track may not be in your library yet."
                : "Type to search your library."}
            </p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
