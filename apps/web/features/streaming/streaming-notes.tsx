"use client";

import {
  creditLines,
  type StreamingAlbum,
  type StreamingArtistSummary,
  type StreamingEditorialText,
} from "@parson/music-sdk";
import { Award, UserRound } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useState } from "react";
import { editorialParagraphs } from "./streaming-model";

/** A review or biography: a few paragraphs, then "Read more". */
export function EditorialBlock({
  title,
  text,
}: {
  title: string;
  text: StreamingEditorialText;
}) {
  const [expanded, setExpanded] = useState(false);
  const paragraphs = editorialParagraphs(text.text);
  const long = paragraphs.length > 2 || text.text.length > 600;
  const shown = expanded || !long ? paragraphs : paragraphs.slice(0, 1);
  return (
    <section className="mt-12 max-w-3xl">
      <h2 className="mb-3 text-lg font-semibold text-white">{title}</h2>
      <div
        className={!expanded && long ? "line-clamp-6 space-y-3" : "space-y-3"}
      >
        {shown.map((paragraph, index) => (
          <p
            className="whitespace-pre-line text-sm leading-6 text-zinc-300"
            key={index}
          >
            {paragraph}
          </p>
        ))}
      </div>
      <div className="mt-2 flex items-center gap-3 text-xs text-zinc-500">
        {long && (
          <button
            className="font-semibold text-zinc-300 hover:text-white"
            onClick={() => setExpanded((value) => !value)}
            type="button"
          >
            {expanded ? "Show less" : "Read more"}
          </button>
        )}
        {text.source && <span>Source: {text.source}</span>}
      </div>
    </section>
  );
}

export function AwardChips({ awards }: { awards: string[] | undefined }) {
  if (!awards?.length) return null;
  return (
    <div className="mt-4 flex flex-wrap gap-2">
      {awards.map((award) => (
        <span
          className="inline-flex items-center gap-1.5 rounded-full border border-amber-300/25 bg-amber-300/8 px-2.5 py-1 text-xs font-semibold text-amber-200"
          key={award}
        >
          <Award aria-hidden="true" className="h-3.5 w-3.5" />
          {award}
        </span>
      ))}
    </div>
  );
}

/** Per-track credits as the service lists them, one track per row. */
export function AlbumCredits({ album }: { album: StreamingAlbum }) {
  const credited = album.tracks.filter((track) => track.credits?.length);
  const [open, setOpen] = useState(false);
  if (!credited.length) return null;
  const visible = open ? credited : credited.slice(0, 3);
  return (
    <section className="mt-12">
      <h2 className="mb-1 text-lg font-semibold text-white">Credits</h2>
      <p className="mb-4 text-xs text-zinc-500">As listed by the service.</p>
      <dl className="space-y-4">
        {visible.map((track) => (
          <div
            className="grid gap-1 sm:grid-cols-[minmax(0,14rem)_1fr] sm:gap-6"
            key={track.id}
          >
            <dt className="truncate text-sm font-medium text-white">
              {track.track_number}. {track.title}
            </dt>
            <dd className="space-y-0.5 text-sm text-zinc-400">
              {creditLines(track.credits).map((line) => (
                <p key={line}>{line}</p>
              ))}
            </dd>
          </div>
        ))}
      </dl>
      {credited.length > 3 && (
        <button
          className="mt-4 text-xs font-semibold text-zinc-300 hover:text-white"
          onClick={() => setOpen((value) => !value)}
          type="button"
        >
          {open
            ? "Show fewer"
            : `Show credits for all ${credited.length} tracks`}
        </button>
      )}
    </section>
  );
}

export function SimilarArtists({
  artists,
}: {
  artists: StreamingArtistSummary[] | undefined;
}) {
  if (!artists?.length) return null;
  return (
    <section className="mt-12">
      <h2 className="mb-5 text-xl font-semibold text-white">Similar artists</h2>
      <div className="grid grid-cols-3 gap-4 sm:grid-cols-4 lg:grid-cols-6">
        {artists.map((artist) => (
          <Link
            className="group min-w-0 text-center"
            href={`/artist?id=${encodeURIComponent(artist.id)}`}
            key={artist.id}
          >
            <div className="relative mx-auto flex aspect-square w-full items-center justify-center overflow-hidden rounded-full bg-white/5">
              {artist.picture_url ? (
                <Image
                  alt={artist.name}
                  className="object-cover"
                  fill
                  sizes="160px"
                  src={artist.picture_url}
                />
              ) : (
                <UserRound className="h-8 w-8 text-zinc-600" />
              )}
            </div>
            <p className="mt-2 truncate text-sm font-medium text-zinc-200 group-hover:underline">
              {artist.name}
            </p>
          </Link>
        ))}
      </div>
    </section>
  );
}
