"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { ReleaseMatch, ReleaseSearchTerms } from "@parson/music-sdk";
import { Check, ExternalLink, Loader2, Search } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { identifyError, useIdentifyAlbum, useReleaseSearch } from "./use-works";

function releaseFacts(release: ReleaseMatch) {
  return [
    release.date?.slice(0, 4),
    release.country,
    release.format,
    release.track_count != null &&
      `${release.track_count} ${release.track_count === 1 ? "track" : "tracks"}`,
    [release.labels[0], release.catalog_numbers[0]].filter(Boolean).join(" "),
  ].filter(Boolean) as string[];
}

/**
 * Roon's "Identify album": search MusicBrainz and pick the release this
 * album is. The choice wins over tags and automatic matching, so a skipped
 * or wrong match can be fixed by hand.
 */
export function IdentifyAlbumDialog({
  albumId,
  albumName,
  open,
  onOpenChange,
}: {
  albumId: string;
  albumName: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [titleDraft, setTitleDraft] = useState<string | null>(null);
  const [artistDraft, setArtistDraft] = useState<string | null>(null);
  const [terms, setTerms] = useState<ReleaseSearchTerms>({});
  const search = useReleaseSearch(albumId, terms, open);
  const { choose, clear } = useIdentifyAlbum(albumId);
  const current = search.data?.identification?.release_mbid ?? null;
  const error = search.isError ? identifyError(search.error) : null;
  const title = titleDraft ?? search.data?.title ?? "";
  const artist = artistDraft ?? search.data?.artist ?? "";

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-hidden">
        <DialogHeader>
          <DialogTitle>Identify album</DialogTitle>
          <DialogDescription>
            Choose the MusicBrainz release that {albumName} is. Credits, works,
            background and reviews then come from that release.
          </DialogDescription>
        </DialogHeader>
        <form
          className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,0.8fr)_auto]"
          onSubmit={(event) => {
            event.preventDefault();
            setTerms({ title, artist });
          }}
        >
          <Input
            aria-label="Release title"
            onChange={(event) => setTitleDraft(event.target.value)}
            placeholder="Release title"
            value={title}
          />
          <Input
            aria-label="Artist, conductor or performer"
            onChange={(event) => setArtistDraft(event.target.value)}
            placeholder="Artist, conductor or performer"
            value={artist}
          />
          <button
            className="flex h-9 shrink-0 items-center justify-center gap-2 rounded-md bg-white px-3 text-sm font-semibold text-black hover:bg-zinc-200 disabled:opacity-50"
            disabled={search.isFetching || !title.trim()}
            type="submit"
          >
            {search.isFetching ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Search className="h-4 w-4" />
            )}
            Search
          </button>
        </form>
        {search.data?.identification && (
          <div className="flex items-center justify-between gap-3 rounded-md border border-white/10 bg-white/4 px-3 py-2 text-sm">
            <span className="min-w-0 truncate text-zinc-300">
              Matched by hand to{" "}
              <span className="text-white">
                {search.data.identification.release_title || "a release"}
              </span>
              {search.data.identification.release_artist &&
                ` by ${search.data.identification.release_artist}`}
            </span>
            <button
              className="shrink-0 text-xs font-semibold text-zinc-400 hover:text-white disabled:opacity-50"
              disabled={clear.isPending}
              onClick={() => clear.mutate()}
              type="button"
            >
              Use automatic matching
            </button>
          </div>
        )}
        <div className="-mx-2 max-h-[50vh] overflow-y-auto px-2">
          {error && (
            <p className="py-6 text-sm leading-6 text-zinc-400">
              {error.message}{" "}
              {error.disabled && (
                <Link
                  className="text-white underline-offset-2 hover:underline"
                  href="/settings?tab=library"
                >
                  Open settings
                </Link>
              )}
            </p>
          )}
          {search.isPending && !error && (
            <p className="flex items-center gap-2 py-6 text-sm text-zinc-500">
              <Loader2 className="h-4 w-4 animate-spin" /> Searching
              MusicBrainz…
            </p>
          )}
          {search.data && search.data.releases.length === 0 && (
            <p className="py-6 text-sm text-zinc-500">
              No releases found. Try fewer words, the original title, or a
              catalogue number.
            </p>
          )}
          <ul aria-label="MusicBrainz releases" className="space-y-1">
            {search.data?.releases.map((release) => {
              const chosen = current === release.mbid;
              return (
                <li
                  className={`flex items-center gap-3 rounded-md px-3 py-2.5 ${
                    chosen ? "bg-white/8" : "hover:bg-white/4"
                  }`}
                  key={release.mbid}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-medium text-white">
                        {release.title}
                      </span>
                      {release.same_track_count && (
                        <span className="shrink-0 rounded border border-emerald-400/30 px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide text-emerald-300">
                          {search.data?.local_track_count} tracks, like yours
                        </span>
                      )}
                    </div>
                    <p className="truncate text-sm text-zinc-400">
                      {release.artist}
                      {release.disambiguation && ` (${release.disambiguation})`}
                    </p>
                    <p className="truncate text-xs text-zinc-500">
                      {releaseFacts(release).join(" · ")}
                    </p>
                  </div>
                  <a
                    aria-label={`${release.title} on MusicBrainz`}
                    className="shrink-0 rounded p-1.5 text-zinc-500 hover:text-white"
                    href={release.url}
                    rel="noreferrer"
                    target="_blank"
                    title="Open on MusicBrainz"
                  >
                    <ExternalLink className="h-4 w-4" />
                  </a>
                  <button
                    className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-semibold ${
                      chosen
                        ? "bg-emerald-400/15 text-emerald-300"
                        : "bg-white text-black hover:bg-zinc-200"
                    } disabled:opacity-50`}
                    disabled={chosen || choose.isPending}
                    onClick={() =>
                      choose.mutate(release, {
                        onSuccess: () => onOpenChange(false),
                      })
                    }
                    type="button"
                  >
                    {chosen && <Check className="h-3.5 w-3.5" />}
                    {chosen ? "Chosen" : "This is it"}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
        <p className="text-xs text-zinc-500">
          Searches musicbrainz.org. Release data is CC0; Wikipedia text keeps
          its CC BY-SA attribution.
        </p>
      </DialogContent>
    </Dialog>
  );
}
