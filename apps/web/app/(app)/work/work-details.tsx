"use client";

import getBaseURL from "@/lib/api/server-url";
import { AlbumArtFallback, getLibraryImageUrl } from "@/lib/images/image-url";
import {
  groupRecordings,
  workFacts,
  type PersonRef,
  type RecordingGrouping,
  type WorkRecording,
} from "@parson/music-sdk";
import { ExternalLink, Loader2, Pause, Play } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { EditorialText } from "@/features/library/credits/editorial-text";
import { useRouter, useSearchParams } from "next/navigation";
import { useMemo } from "react";
import { usePageTitle } from "@/components/app/title-metadata";
import { usePlayer } from "@/features/player/player-context";
import { useWork } from "@/features/library/works/use-works";

const GROUPINGS: { key: RecordingGrouping; label: string }[] = [
  { key: "album", label: "Album" },
  { key: "conductor", label: "Conductor" },
  { key: "performer", label: "Performer" },
];

function formatDuration(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  return `${minutes}:${rest < 10 ? "0" : ""}${rest}`;
}

function PersonLink({
  person,
  className = "",
}: {
  person: PersonRef;
  className?: string;
}) {
  const label = person.instrument
    ? `${person.name} (${person.instrument})`
    : person.name;
  if (!person.artist_id) return <span className={className}>{label}</span>;
  return (
    <Link
      className={`hover:text-white hover:underline ${className}`}
      href={`/artist?id=${encodeURIComponent(person.artist_id)}`}
    >
      {label}
    </Link>
  );
}

function PeopleLine({ label, people }: { label: string; people: PersonRef[] }) {
  if (!people.length) return null;
  return (
    <div className="flex min-w-0 gap-2 text-sm">
      <dt className="w-20 shrink-0 text-zinc-500">{label}</dt>
      <dd className="min-w-0 text-zinc-200">
        {people.map((person, index) => (
          <span key={`${person.artist_id ?? person.name}-${index}`}>
            {index > 0 && ", "}
            <PersonLink person={person} />
          </span>
        ))}
      </dd>
    </div>
  );
}

/** Strips the work title from a movement title: "…, Op. 125: IV. Presto". */
function movementTitle(workTitle: string, title: string) {
  if (!title.startsWith(workTitle)) return title;
  return title.slice(workTitle.length).replace(/^[:\s,–-]+/, "") || title;
}

function RecordingCard({
  recording,
  partCount,
}: {
  recording: WorkRecording;
  partCount: number;
}) {
  const player = usePlayer();
  const cover = getLibraryImageUrl(recording.album.cover_url, getBaseURL);
  const playing =
    player.isPlaying &&
    recording.tracks.some((track) => track.song.id === player.song?.id);
  const play = (index: number) => {
    const target = recording.tracks[index];
    if (!target) return;
    if (player.song?.id === target.song.id) {
      player.togglePlayPause();
      return;
    }
    player.setQueue(
      recording.tracks.map((track) => ({
        song: track.song,
        album: recording.album,
        artist: recording.artist,
      })),
    );
    player.setCurrentSongIndex(index);
    player.setSongCallback(target.song, recording.artist, recording.album);
    player.playAudioSource();
  };
  const coverage =
    partCount > 1
      ? recording.complete
        ? "Complete"
        : `${recording.tracks.length} of ${partCount} movements`
      : null;
  return (
    <article className="grid gap-5 rounded-xl border border-white/8 bg-white/2 p-4 sm:grid-cols-[8.5rem_minmax(0,1fr)] sm:p-5">
      <Link
        aria-label={recording.album.name}
        className="relative block aspect-square w-28 overflow-hidden rounded-md border border-white/10 bg-white/4 sm:w-34"
        href={`/album?id=${recording.album.id}`}
      >
        {cover ? (
          <Image
            alt={recording.album.name}
            className="object-cover"
            fill
            sizes="136px"
            src={cover}
          />
        ) : (
          <AlbumArtFallback />
        )}
      </Link>
      <div className="min-w-0">
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1">
            <Link
              className="line-clamp-2 text-lg font-semibold leading-6 text-white hover:underline"
              href={`/album?id=${recording.album.id}`}
            >
              {recording.album.name}
            </Link>
            <p className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-zinc-400">
              <span>
                {recording.artist.name}
                {recording.year && ` · ${recording.year}`}
                {` · ${formatDuration(recording.duration)}`}
              </span>
              {coverage && (
                <span
                  className={`rounded border px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide ${
                    recording.complete
                      ? "border-white/15 text-zinc-300"
                      : "border-amber-300/30 text-amber-200"
                  }`}
                >
                  {coverage}
                </span>
              )}
            </p>
          </div>
          <button
            aria-label={`${playing ? "Pause" : "Play"} ${recording.album.name}`}
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-white text-black transition-transform hover:scale-105"
            onClick={() => play(0)}
            type="button"
          >
            {playing ? (
              <Pause className="h-5 w-5 fill-black" />
            ) : (
              <Play className="ml-0.5 h-5 w-5 fill-black" />
            )}
          </button>
        </div>
        <dl className="mt-3 space-y-1">
          <PeopleLine label="Conductor" people={recording.conductors} />
          <PeopleLine label="Ensemble" people={recording.ensembles} />
          <PeopleLine label="Soloists" people={recording.soloists} />
          <PeopleLine label="Artists" people={recording.artists} />
        </dl>
        <ol className="mt-3 divide-y divide-white/5">
          {recording.tracks.map((track, index) => (
            <li key={track.song.id}>
              <button
                className={`grid w-full grid-cols-[minmax(0,1fr)_3.5rem] gap-3 rounded px-2 py-1.5 text-left text-sm transition-colors hover:bg-white/5 ${
                  player.song?.id === track.song.id
                    ? "text-white"
                    : "text-zinc-300"
                }`}
                onClick={() => play(index)}
                type="button"
              >
                <span className="truncate">{track.part_title}</span>
                <span className="text-right tabular-nums text-zinc-500">
                  {formatDuration(track.song.duration)}
                </span>
              </button>
            </li>
          ))}
        </ol>
      </div>
    </article>
  );
}

/**
 * A composition: who wrote it, its form, key and catalogue number, its
 * movements, and every recording of it in the library, grouped by album,
 * conductor or performer.
 */
export default function WorkDetails() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const id = searchParams?.get("id");
  const grouping: RecordingGrouping =
    GROUPINGS.find((entry) => entry.key === searchParams?.get("by"))?.key ??
    "album";
  const query = useWork(id);
  const page = query.data;
  usePageTitle(page ? page.work.title : null);
  const groups = useMemo(
    () => (page ? groupRecordings(page.recordings, grouping) : []),
    [grouping, page],
  );

  if (!id) {
    return (
      <p className="mx-auto max-w-225 px-7 pt-16 text-zinc-400">
        Open a work from a track menu or a composer&apos;s Compositions.
      </p>
    );
  }
  if (query.isPending) {
    return (
      <div className="flex justify-center pt-24 text-zinc-500">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }
  if (query.isError || !page) {
    return (
      <div className="mx-auto max-w-225 px-7 pt-16 text-zinc-400">
        <p>This work is not in your library.</p>
        <button
          className="mt-3 text-sm text-white underline-offset-2 hover:underline"
          onClick={() => void query.refetch()}
          type="button"
        >
          Try again
        </button>
      </div>
    );
  }

  const { work, parent, parts } = page;
  const facts = workFacts(work);
  const setGrouping = (key: RecordingGrouping) => {
    const params = new URLSearchParams(searchParams?.toString());
    if (key === "album") params.delete("by");
    else params.set("by", key);
    router.replace(`/work?${params.toString()}`, { scroll: false });
  };

  return (
    <div className="tidal-route relative text-zinc-50">
      <div className="relative mx-auto max-w-225 px-5 pb-24 pt-10 sm:px-7">
        <header className="mb-10">
          <p className="mb-3 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
            {parent ? "Movement" : "Composition"}
          </p>
          <h1 className="text-balance text-4xl font-black leading-[1.02] text-white sm:text-5xl">
            {work.title}
          </h1>
          {work.composer && (
            <p className="mt-4 text-lg font-semibold text-zinc-200">
              <PersonLink person={work.composer} />
            </p>
          )}
          {facts.length > 0 && (
            <ul
              aria-label="About this work"
              className="mt-5 flex flex-wrap gap-2"
            >
              {facts.map((fact) => (
                <li
                  className="rounded-full bg-white/6 px-3 py-1 text-xs font-medium text-zinc-300"
                  key={fact}
                >
                  {fact}
                </li>
              ))}
            </ul>
          )}
          <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-2 text-sm">
            {parent && (
              <Link
                className="text-zinc-300 hover:text-white hover:underline"
                href={`/work?id=${encodeURIComponent(parent.mbid)}`}
              >
                Part of {parent.title}
              </Link>
            )}
            <a
              className="inline-flex items-center gap-1.5 text-zinc-500 hover:text-white"
              href={page.url}
              rel="noreferrer"
              target="_blank"
            >
              MusicBrainz <ExternalLink className="h-3.5 w-3.5" />
            </a>
          </div>
        </header>

        {page.about && (
          <section aria-labelledby="work-about" className="mb-12 max-w-170">
            <h2
              className="mb-3 text-sm font-semibold uppercase tracking-[0.08em] text-zinc-400"
              id="work-about"
            >
              About
            </h2>
            <EditorialText clampLines={5} text={page.about} />
          </section>
        )}

        {parts.length > 0 && (
          <section aria-labelledby="work-movements" className="mb-12">
            <h2
              className="mb-3 text-sm font-semibold uppercase tracking-[0.08em] text-zinc-400"
              id="work-movements"
            >
              Movements
            </h2>
            <ol className="grid gap-x-8 gap-y-1 text-sm text-zinc-300 sm:grid-cols-2">
              {parts.map((part) => (
                <li className="truncate" key={part.mbid}>
                  <Link
                    className="hover:text-white hover:underline"
                    href={`/work?id=${encodeURIComponent(part.mbid)}`}
                  >
                    {movementTitle(work.title, part.title)}
                  </Link>
                </li>
              ))}
            </ol>
          </section>
        )}

        <section aria-labelledby="work-recordings">
          <div className="mb-6 flex flex-wrap items-end justify-between gap-4 border-b border-white/10 pb-3">
            <h2 className="text-2xl font-bold text-white" id="work-recordings">
              Recordings
              <span className="ml-2 text-base font-medium text-zinc-500">
                {page.recordings.length}
              </span>
            </h2>
            <div
              aria-label="Group recordings by"
              className="flex items-center gap-1 rounded-full bg-white/5 p-1"
              role="radiogroup"
            >
              {GROUPINGS.map((entry) => (
                <button
                  aria-checked={grouping === entry.key}
                  className={`rounded-full px-3 py-1 text-xs font-semibold transition-colors ${
                    grouping === entry.key
                      ? "bg-white text-black"
                      : "text-zinc-400 hover:text-white"
                  }`}
                  key={entry.key}
                  onClick={() => setGrouping(entry.key)}
                  role="radio"
                  type="button"
                >
                  {entry.label}
                </button>
              ))}
            </div>
          </div>
          {page.recordings.length === 0 && (
            <p className="text-sm text-zinc-500">
              No track in your library performs this work yet.
            </p>
          )}
          <div className="space-y-10">
            {groups.map((group) => (
              <div key={group.key}>
                {grouping !== "album" && (
                  <h3 className="mb-3 text-base font-semibold text-zinc-200">
                    {group.person ? (
                      <PersonLink person={group.person} />
                    ) : (
                      <span className="text-zinc-500">{group.label}</span>
                    )}
                    <span className="ml-2 text-sm font-medium text-zinc-500">
                      {group.recordings.length}{" "}
                      {group.recordings.length === 1
                        ? "recording"
                        : "recordings"}
                    </span>
                  </h3>
                )}
                <div className="space-y-4">
                  {group.recordings.map((recording) => (
                    <RecordingCard
                      key={`${group.key}-${recording.album.id}`}
                      partCount={parts.length}
                      recording={recording}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}
