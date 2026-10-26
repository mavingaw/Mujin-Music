"use client";

import type { LibraryAlbum } from "@parson/music-sdk";
import {
  albumTextPanels,
  resolvedFact,
  enrichmentWaitingLabel,
  releaseMatchFact,
  releaseMatchSignals,
  releaseMatchSummary,
  releaseYearDiffers,
  UNVERIFIED_MATCH_TEXT,
  unverifiedMatch,
  workHref,
} from "@parson/music-sdk";
import type {
  AlbumAbout,
  ReleaseMatchCheck,
  ResolvedFact,
} from "@parson/music-sdk/types";
import {
  Check,
  CircleHelp,
  ExternalLink,
  ShieldAlert,
  SlidersHorizontal,
  X,
} from "lucide-react";
import { FactSourceBadge } from "@/features/library/credits/fact-source-badge";
import Link from "next/link";
import { useState } from "react";
import { EditorialText } from "@/features/library/credits/editorial-text";
import { NoteEditor } from "@/features/library/credits/note-editor";
import {
  albumFormat,
  formatPartialDate,
} from "@/features/library/credits/credit-roles";

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

type Fact = { label: string; value: string; source?: ResolvedFact };

/** Facts with their sources (tags, MusicBrainz or a frozen edit). */
function resolvedFacts(album: LibraryAlbum, about: AlbumAbout): Fact[] {
  const fact = (field: ResolvedFact["field"]) => resolvedFact(about, field);
  const original = fact("original_date");
  const edition = fact("release_date");
  const shown = (
    label: string,
    source: ResolvedFact | null,
    format: (value: string) => string | null | undefined = (value) => value,
  ): Fact | null => {
    const value = source?.value ? format(source.value) : null;
    return value ? { label, value, source: source ?? undefined } : null;
  };
  const length = album.songs.reduce(
    (total, song) => total + (song.duration || 0),
    0,
  );
  return [
    shown("Released", original, formatPartialDate),
    // This edition only when it differs from the original release.
    edition?.value && edition.value !== original?.value
      ? shown(
          original?.value ? "This edition" : "Released",
          edition,
          formatPartialDate,
        )
      : null,
    shown("Label", fact("label")),
    shown("Catalog no.", fact("catalog_number")),
    shown("Country", fact("country"), countryName),
    shown("Barcode", fact("barcode")),
    length > 0 ? { label: "Length", value: formatLength(length) } : null,
    albumFormat(album.songs)
      ? { label: "Format", value: albumFormat(album.songs) as string }
      : null,
    releaseMatchFact(about)
      ? { label: "Matched", value: releaseMatchFact(about) as string }
      : null,
  ].filter(Boolean) as Fact[];
}

export function albumFacts(
  album: LibraryAlbum,
  about: AlbumAbout | undefined,
): Fact[] {
  if (about?.sources) return resolvedFacts(album, about);
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
  // A release from another year than the tags (a later CD of a 1975
  // recording) is this edition, not the original release.
  const released =
    exact && !releaseYearDiffers(about)
      ? (formatPartialDate(facts?.first_release_date) ??
        formatPartialDate(album.first_release_date))
      : (formatPartialDate(album.first_release_date) ??
        formatPartialDate(facts?.first_release_date));
  const edition =
    exact &&
    facts?.date &&
    (facts.date !== facts.first_release_date || releaseYearDiffers(about))
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
    releaseMatchFact(about) && {
      label: "Matched",
      value: releaseMatchFact(about) as string,
    },
  ].filter(Boolean) as { label: string; value: string }[];
}

const SIGNAL_STYLE = {
  match: { icon: Check, className: "text-emerald-300" },
  partial: { icon: CircleHelp, className: "text-amber-200" },
  mismatch: { icon: X, className: "text-rose-300" },
  unknown: { icon: CircleHelp, className: "text-zinc-500" },
} as const;

/**
 * Shown when the closest MusicBrainz release failed the match check: the
 * album keeps its tags, and the evidence says why, so a person can decide.
 */
export function UnverifiedMatchBanner({
  check,
  isAdmin,
  onIdentify,
}: {
  check: ReleaseMatchCheck;
  isAdmin: boolean;
  onIdentify?: () => void;
}) {
  const summary = releaseMatchSummary(check);
  return (
    <div
      aria-label="Unverified match"
      className="mb-7 overflow-hidden rounded-xl border border-amber-300/20 bg-gradient-to-br from-amber-300/[0.09] via-amber-300/[0.04] to-transparent"
      role="status"
    >
      <div className="flex flex-col gap-4 p-4 sm:flex-row sm:items-start sm:p-5">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-amber-300/15 text-amber-200">
          <ShieldAlert className="h-[18px] w-[18px]" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-amber-200">
            Unverified match
          </p>
          <p className="mt-1 text-sm leading-6 text-zinc-200">
            Closest release on MusicBrainz:{" "}
            <a
              className="font-semibold text-white underline-offset-2 hover:underline"
              href={check.url}
              rel="noreferrer"
              target="_blank"
            >
              {check.release_title}
            </a>
            {check.release_artist && (
              <span className="text-zinc-400"> · {check.release_artist}</span>
            )}
            {summary && <span className="text-zinc-400"> · {summary}</span>}
          </p>
          <p className="mt-1 text-sm leading-6 text-zinc-400">
            {UNVERIFIED_MATCH_TEXT}
          </p>
          <ul aria-label="Match evidence" className="mt-3 grid gap-1.5">
            {releaseMatchSignals(check).map((signal) => {
              const style =
                SIGNAL_STYLE[signal.outcome] ?? SIGNAL_STYLE.unknown;
              const Icon = style.icon;
              return (
                <li
                  className="flex items-start gap-2 text-[13px] leading-5 text-zinc-300"
                  key={signal.key}
                >
                  <Icon
                    aria-label={signal.outcome}
                    className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${style.className}`}
                  />
                  <span className="min-w-0">{signal.detail}</span>
                </li>
              );
            })}
          </ul>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2 sm:flex-col sm:items-stretch">
          {isAdmin && onIdentify ? (
            <button
              className="rounded-full bg-white px-4 py-2 text-xs font-semibold text-black transition-colors hover:bg-zinc-200"
              onClick={onIdentify}
              type="button"
            >
              Identify album…
            </button>
          ) : (
            <p className="max-w-[14rem] text-xs leading-5 text-zinc-500">
              An administrator can identify this album.
            </p>
          )}
          <a
            className="flex items-center justify-center gap-1.5 rounded-full border border-white/12 px-4 py-2 text-xs font-semibold text-zinc-300 transition-colors hover:border-white/25 hover:text-white"
            href={check.url}
            rel="noreferrer"
            target="_blank"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            MusicBrainz
          </a>
        </div>
      </div>
    </div>
  );
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
  onEditSources,
}: {
  album: LibraryAlbum;
  about: AlbumAbout | undefined;
  isAdmin: boolean;
  /** Opens "Identify album" (administrators). */
  onIdentify?: () => void;
  /** Opens the album's metadata preference (administrators). */
  onEditSources?: () => void;
}) {
  const textPanels = albumTextPanels(about);
  const panels: { key: string; label: string }[] = [
    ...textPanels.map(({ key, label }) => ({ key, label })),
    { key: "note", label: about?.note ? "Your note" : "Add a note" },
  ];
  const [selected, setSelected] = useState<string | null>(null);
  const active =
    panels.find((panel) => panel.key === selected)?.key ??
    panels[0]?.key ??
    "note";
  const activeText = textPanels.find((panel) => panel.key === active);
  const facts = albumFacts(album, about);
  const hasEditorial = Boolean(textPanels.length || about?.note);
  const unverified = unverifiedMatch(about);

  return (
    <section
      aria-label="About this album"
      className="mb-9 grid gap-x-8 md:grid-cols-[minmax(0,1fr)_17rem]"
    >
      {unverified && (
        <div className="md:col-span-2">
          <UnverifiedMatchBanner
            check={unverified}
            isAdmin={isAdmin}
            onIdentify={onIdentify}
          />
        </div>
      )}
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
          {activeText && (
            <div className="space-y-3">
              {activeText.workMbid && (
                <p className="text-xs font-semibold uppercase tracking-[0.12em] text-zinc-500">
                  About the piece ·{" "}
                  <Link
                    className="text-zinc-300 normal-case tracking-normal underline-offset-2 hover:text-white hover:underline"
                    href={workHref(activeText.workMbid)}
                  >
                    {activeText.workTitle}
                  </Link>
                </p>
              )}
              <EditorialText key={activeText.key} text={activeText.text} />
            </div>
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
              {!hasEditorial &&
                about?.enrichment.enabled &&
                about.enrichment.task?.status === "pending" && (
                  <p className="text-sm leading-6 text-zinc-500">
                    {enrichmentWaitingLabel(about.enrichment.task, "album")}
                  </p>
                )}
              {isAdmin &&
                onIdentify &&
                about?.enrichment.enabled &&
                !about.identification &&
                !unverified &&
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
        <dl className="mt-8 grid h-fit grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 text-sm md:mt-0 md:border-l md:border-white/8 md:pl-6">
          {facts.map((fact) => (
            <div className="contents" key={fact.label}>
              <dt className="font-medium text-zinc-500">{fact.label}</dt>
              <dd className="flex min-w-0 items-center gap-1.5 text-zinc-200">
                <span className="min-w-0 truncate" title={fact.value}>
                  {fact.value}
                </span>
                {fact.source && <FactSourceBadge compact fact={fact.source} />}
              </dd>
            </div>
          ))}
          {onEditSources && (
            <div className="col-span-2 mt-2">
              <button
                className="flex items-center gap-1.5 text-xs font-semibold text-zinc-400 transition-colors hover:text-white"
                onClick={onEditSources}
                type="button"
              >
                <SlidersHorizontal className="h-3.5 w-3.5" />
                Tags or MusicBrainz, field by field…
              </button>
            </div>
          )}
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
