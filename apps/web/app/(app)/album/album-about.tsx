"use client";

import type { LibraryAlbum } from "@parson/music-sdk";
import type { AlbumAbout } from "@parson/music-sdk/types";
import Link from "next/link";
import { useState } from "react";
import { EditorialText } from "@/features/library/credits/editorial-text";
import { NoteEditor } from "@/features/library/credits/note-editor";
import {
  albumFormat,
  formatPartialDate,
} from "@/features/library/credits/credit-roles";

type Panel = "review" | "background" | "note";

function formatLength(seconds: number) {
  const minutes = Math.max(1, Math.round(seconds / 60));
  if (minutes < 60) return `${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
  return `${Math.floor(minutes / 60)} hr ${minutes % 60} min`;
}

/** MusicBrainz uses ISO regions plus XE (Europe) and XW (worldwide). */
function countryName(code: string) {
  if (code === "XE") return "Europe";
  if (code === "XW") return "Worldwide";
  try {
    return new Intl.DisplayNames(["en"], { type: "region" }).of(code) ?? code;
  } catch {
    return code;
  }
}

export function albumFacts(album: LibraryAlbum, about: AlbumAbout | undefined) {
  const facts = about?.facts;
  // Edition facts only describe this album when the release matched exactly.
  const exact = facts?.exact === true;
  const labels = exact ? (facts?.labels ?? []) : [];
  const label = album.label ?? labels[0]?.name ?? null;
  const catalog = labels
    .map((entry) => entry.catalog_number)
    .filter((value): value is string => Boolean(value && value !== "[none]"))
    .join(", ");
  // A release matched by title alone may be a later recording of the same
  // programme; then the year in the tags wins.
  const released = exact
    ? (formatPartialDate(facts?.first_release_date) ??
      formatPartialDate(album.first_release_date))
    : (formatPartialDate(album.first_release_date) ??
      formatPartialDate(facts?.first_release_date));
  const edition =
    exact && facts?.date && facts.date !== facts.first_release_date
      ? formatPartialDate(facts.date)
      : null;
  const length = album.songs.reduce(
    (total, song) => total + (song.duration || 0),
    0,
  );
  return [
    released && { label: "Released", value: released },
    edition && { label: "This edition", value: edition },
    label && { label: "Label", value: label },
    catalog && { label: "Catalog no.", value: catalog },
    exact &&
      facts?.country && { label: "Country", value: countryName(facts.country) },
    length > 0 && { label: "Length", value: formatLength(length) },
    albumFormat(album.songs) && {
      label: "Format",
      value: albumFormat(album.songs) as string,
    },
    about?.identification && {
      label: "Matched",
      value: "By hand on MusicBrainz",
    },
  ].filter(Boolean) as { label: string; value: string }[];
}

/**
 * The block under the album hero: review, background and the listener's
 * own note on the left; release facts on the right.
 */
export function AlbumAboutSection({
  album,
  about,
  isAdmin,
  onIdentify,
}: {
  album: LibraryAlbum;
  about: AlbumAbout | undefined;
  isAdmin: boolean;
  /** Opens "Identify album" (administrators). */
  onIdentify?: () => void;
}) {
  const panels: { key: Panel; label: string }[] = [
    about?.review ? { key: "review" as const, label: "Review" } : null,
    about?.background ? { key: "background" as const, label: "About" } : null,
    { key: "note" as const, label: about?.note ? "Your note" : "Add a note" },
  ].filter((panel): panel is { key: Panel; label: string } => Boolean(panel));
  const [selected, setSelected] = useState<Panel | null>(null);
  const active =
    panels.find((panel) => panel.key === selected)?.key ??
    panels[0]?.key ??
    "note";
  const facts = albumFacts(album, about);
  const hasEditorial = Boolean(
    about?.review || about?.background || about?.note,
  );

  return (
    <section
      aria-label="About this album"
      className="mb-9 grid gap-8 md:grid-cols-[minmax(0,1fr)_17rem]"
    >
      <div className="min-w-0">
        <div
          aria-label="Album text"
          className="mb-4 flex flex-wrap items-center gap-1"
          role="tablist"
        >
          {panels.map((panel) => (
            <button
              aria-selected={active === panel.key}
              className={`rounded-full px-3 py-1.5 text-xs font-semibold transition-colors ${
                active === panel.key
                  ? "bg-white text-black"
                  : "bg-white/6 text-zinc-300 hover:bg-white/10 hover:text-white"
              }`}
              key={panel.key}
              onClick={() => setSelected(panel.key)}
              role="tab"
              type="button"
            >
              {panel.label}
            </button>
          ))}
        </div>
        <div role="tabpanel">
          {active === "review" && about?.review && (
            <EditorialText key="review" text={about.review} />
          )}
          {active === "background" && about?.background && (
            <EditorialText key="background" text={about.background} />
          )}
          {active === "note" && (
            <div className="space-y-4">
              {!hasEditorial && about && !about.enrichment.enabled && (
                <p className="text-sm leading-6 text-zinc-500">
                  {isAdmin ? (
                    <>
                      Album background and reviews come from Wikipedia when
                      online metadata is on.{" "}
                      <Link
                        className="text-zinc-300 underline-offset-2 hover:text-white hover:underline"
                        href="/settings?tab=library"
                      >
                        Turn it on in Settings
                      </Link>
                      .
                    </>
                  ) : (
                    "Album background and reviews appear here when your server administrator turns on online metadata."
                  )}
                </p>
              )}
              {isAdmin &&
                onIdentify &&
                about?.enrichment.enabled &&
                !about.identification &&
                about.enrichment.task?.status === "skipped" && (
                  <p className="rounded-md border border-white/8 bg-white/3 px-3 py-2.5 text-sm leading-6 text-zinc-400">
                    Parson found no confident MusicBrainz match for this album
                    {about.enrichment.task.last_error
                      ? ` (${about.enrichment.task.last_error})`
                      : ""}
                    .{" "}
                    <button
                      className="font-semibold text-white underline-offset-2 hover:underline"
                      onClick={onIdentify}
                      type="button"
                    >
                      Identify album…
                    </button>
                  </p>
                )}
              <NoteEditor entityType="album" id={album.id} note={about?.note} />
            </div>
          )}
        </div>
      </div>
      {facts.length > 0 && (
        <dl className="grid h-fit grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 text-sm md:border-l md:border-white/8 md:pl-6">
          {facts.map((fact) => (
            <div className="contents" key={fact.label}>
              <dt className="font-medium text-zinc-500">{fact.label}</dt>
              <dd className="min-w-0 text-zinc-200">{fact.value}</dd>
            </div>
          ))}
          {about?.links.map((link) => (
            <div className="contents" key={link.url}>
              <dt className="font-medium text-zinc-500">
                {link.provider === "musicbrainz" ? "MusicBrainz" : "Wikidata"}
              </dt>
              <dd>
                <a
                  className="text-zinc-300 underline-offset-2 hover:text-white hover:underline"
                  href={link.url}
                  rel="noreferrer"
                  target="_blank"
                >
                  View
                </a>
              </dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}
