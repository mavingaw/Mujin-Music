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
import { filterIdentifyReleases, trackCountWarning } from "@parson/music-sdk";
import {
  AlertTriangle,
  Check,
  ExternalLink,
  Loader2,
  Search,
} from "lucide-react";
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
  // A release whose track count differs waits for a second click.
  const [confirming, setConfirming] = useState<string | null>(null);
  const localTrackCount = search.data?.local_track_count ?? 0;
  const artist = artistDraft ?? search.data?.artist ?? "";
  // Narrow results to releases crediting the tagged composer/conductor.
  const [only, setOnly] = useState<{ composer?: boolean; conductor?: boolean }>(
    {},
  );
  const composers = search.data?.tagged_composers ?? [];
  const conductors = search.data?.tagged_conductors ?? [];
  const releases = search.data
    ? filterIdentifyReleases(search.data.releases, only)
    : [];
  const hidden = (search.data?.releases.length ?? 0) - releases.length;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      {/* One shrinkable column: long release titles truncate instead of
          pushing the buttons past the dialog's edge. */}
      <DialogContent className="max-h-[85vh] max-w-2xl grid-cols-[minmax(0,1fr)] overflow-hidden">
        <DialogHeader>
          <DialogTitle>Identify album</DialogTitle>
          <DialogDescription>
            Choose the MusicBrainz release that {albumName} is. Credits, works,
            background and reviews then come from that release. Fields you froze
            and credits you edited are kept.
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
        {(composers.length > 0 || conductors.length > 0) && (
          <div
            aria-label="Tagged people"
            className="flex flex-wrap items-center gap-1.5 text-xs"
          >
            <span className="mr-1 text-zinc-500">
              Releases crediting your tags come first. Show only:
            </span>
            {(
              [
                ["composer", composers],
                ["conductor", conductors],
              ] as const
            )
              .filter(([, names]) => names.length > 0)
              .map(([key, names]) => (
                <button
                  aria-pressed={Boolean(only[key])}
                  className={`rounded-full border px-2.5 py-1 font-semibold transition-colors ${
                    only[key]
                      ? "border-emerald-300/40 bg-emerald-300/15 text-emerald-200"
                      : "border-white/12 text-zinc-300 hover:border-white/25 hover:text-white"
                  }`}
                  key={key}
                  onClick={() =>
                    setOnly((current) => ({ ...current, [key]: !current[key] }))
                  }
                  type="button"
                >
                  {key === "composer" ? "Composer" : "Conductor"}{" "}
                  {names.slice(0, 2).join(", ")}
                </button>
              ))}
            {hidden > 0 && (
              <span className="text-zinc-500">
                {hidden} {hidden === 1 ? "release" : "releases"} hidden
              </span>
            )}
          </div>
        )}
        {search.data?.identification && (
          <div className="flex min-w-0 items-center justify-between gap-3 rounded-md border border-white/10 bg-white/4 px-3 py-2 text-sm">
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
          {search.data &&
            search.data.releases.length > 0 &&
            releases.length === 0 && (
              <p className="py-6 text-sm text-zinc-500">
                No release here credits them. Turn the filter off to see all
                results.
              </p>
            )}
          {search.data && search.data.releases.length === 0 && (
            <p className="py-6 text-sm text-zinc-500">
              No releases found. Mujin Music already searches by each word of
              the title; try the original title, the conductor or performer, or
              a catalogue number.
            </p>
          )}
          <ul aria-label="MusicBrainz releases" className="space-y-1">
            {releases.map((release) => {
              const chosen = current === release.mbid;
              const differs =
                release.track_count_differs ??
                (release.track_count != null &&
                  release.track_count !== localTrackCount);
              const warning = differs
                ? trackCountWarning(release, localTrackCount)
                : null;
              const pick = () =>
                choose.mutate(release, {
                  onSuccess: () => onOpenChange(false),
                });
              return (
                <li
                  className={`rounded-md px-3 py-2.5 ${
                    chosen
                      ? "bg-white/8"
                      : confirming === release.mbid
                        ? "bg-amber-300/[0.07]"
                        : "hover:bg-white/4"
                  }`}
                  key={release.mbid}
                >
                  <div className="flex items-center gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex min-w-0 items-center gap-2">
                        <span className="truncate font-medium text-white">
                          {release.title}
                        </span>
                        {release.same_track_count && (
                          <span className="shrink-0 rounded border border-emerald-400/30 px-1.5 py-px text-[0.625rem] font-semibold uppercase tracking-wide text-emerald-300">
                            {localTrackCount} tracks, like yours
                          </span>
                        )}
                        {differs && (
                          <span className="shrink-0 rounded border border-amber-300/30 px-1.5 py-px text-[0.625rem] font-semibold uppercase tracking-wide text-amber-200">
                            {release.track_count} tracks, yours has{" "}
                            {localTrackCount}
                          </span>
                        )}
                        {release.fit === "likely" && !release.suggested && (
                          <span className="hidden shrink-0 rounded bg-white/8 px-1.5 py-px text-[0.625rem] font-semibold uppercase tracking-wide text-zinc-300 sm:inline">
                            Likely match
                          </span>
                        )}
                        {release.suggested && (
                          <span
                            className="hidden shrink-0 rounded bg-amber-300/10 px-1.5 py-px text-[0.625rem] font-semibold uppercase tracking-wide text-amber-200 sm:inline"
                            title="Automatic matching found this release, but it did not pass the match check."
                          >
                            Failed the check
                          </span>
                        )}
                      </div>
                      {(release.credits_tagged_composer ||
                        release.credits_tagged_conductor) && (
                        <p className="mt-0.5 flex flex-wrap gap-1">
                          {release.credits_tagged_composer && (
                            <span className="rounded bg-emerald-300/10 px-1.5 py-px text-[0.625rem] font-semibold uppercase tracking-wide text-emerald-200">
                              Credits {composers[0] ?? "your composer"}
                            </span>
                          )}
                          {release.credits_tagged_conductor && (
                            <span className="rounded bg-emerald-300/10 px-1.5 py-px text-[0.625rem] font-semibold uppercase tracking-wide text-emerald-200">
                              Credits {conductors[0] ?? "your conductor"}
                            </span>
                          )}
                        </p>
                      )}
                      <p className="truncate text-sm text-zinc-400">
                        {release.artist}
                        {release.disambiguation &&
                          ` (${release.disambiguation})`}
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
                      disabled={
                        chosen ||
                        choose.isPending ||
                        confirming === release.mbid
                      }
                      onClick={() =>
                        warning ? setConfirming(release.mbid) : pick()
                      }
                      type="button"
                    >
                      {chosen && <Check className="h-3.5 w-3.5" />}
                      {chosen ? "Chosen" : "This is it"}
                    </button>
                  </div>
                  {confirming === release.mbid && warning && (
                    <div
                      className="mt-2.5 flex flex-col gap-2.5 rounded-md border border-amber-300/25 bg-black/30 px-3 py-2.5 sm:flex-row sm:items-center"
                      role="alert"
                    >
                      <p className="flex min-w-0 flex-1 items-start gap-2 text-[0.8125rem] leading-5 text-amber-100">
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-300" />
                        {warning}
                      </p>
                      <div className="flex shrink-0 gap-2">
                        <button
                          className="rounded-full px-3 py-1.5 text-xs font-semibold text-zinc-300 hover:text-white"
                          onClick={() => setConfirming(null)}
                          type="button"
                        >
                          Cancel
                        </button>
                        <button
                          className="rounded-full bg-amber-200 px-3 py-1.5 text-xs font-semibold text-black hover:bg-amber-100 disabled:opacity-50"
                          disabled={choose.isPending}
                          onClick={pick}
                          type="button"
                        >
                          Use it anyway
                        </button>
                      </div>
                    </div>
                  )}
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
