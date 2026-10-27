"use client";

import getBaseURL from "@/lib/api/server-url";
import { AlbumArtFallback, getLibraryImageUrl } from "@/lib/images/image-url";
import {
  discographySections,
  focusRoleLabel,
  qualityBadge,
  type DiscographyFocusAlbum,
  type FocusSelection,
  type PersonDiscography,
} from "@parson/music-sdk";
import Image from "next/image";
import Link from "next/link";
import { useMemo } from "react";
import AlbumMenu from "@/features/library/album-menu";
import { BannedBadge } from "@/features/library/radio-bans";
import { FocusPanel } from "@/features/library/focus/discography-focus-panel";
import { cn } from "@/lib/cn";

function QualityBadge({ album }: { album: DiscographyFocusAlbum }) {
  const badge = qualityBadge(album);
  if (!badge) return null;
  return (
    <span
      className={cn(
        "rounded px-1.5 py-px text-[10px] font-bold tabular-nums tracking-wide",
        album.quality === "hires"
          ? "bg-amber-300/15 text-amber-200 ring-1 ring-amber-300/30"
          : album.quality === "lossless"
            ? "bg-white/8 text-zinc-300 ring-1 ring-white/12"
            : "bg-white/5 text-zinc-500 ring-1 ring-white/8",
      )}
      title={album.format ?? undefined}
    >
      {album.quality === "hires" ? `HI-RES ${badge}` : badge}
    </span>
  );
}

function AlbumTile({
  entry,
  personId,
}: {
  entry: DiscographyFocusAlbum;
  personId: string;
}) {
  const cover = getLibraryImageUrl(entry.album.cover_url, getBaseURL);
  const performers = [...entry.conductors, ...entry.ensembles]
    .map((person) => person.name)
    .filter((name) => name !== entry.artist.name)
    .slice(0, 2);
  const ownRoles = entry.roles.filter(
    (role) => role !== "main" && role !== "featured",
  );
  return (
    <AlbumMenu
      album_id={entry.album.id}
      album_name={entry.album.name}
      artist_id={entry.artist.id}
      showArtist={entry.artist.id !== personId}
    >
      <Link
        className="group min-w-0 cursor-pointer"
        href={`/album?id=${entry.album.id}`}
      >
        <div className="relative aspect-square w-full overflow-hidden rounded-lg border border-white/10 bg-white/4 shadow-[0_14px_30px_-18px_rgba(0,0,0,0.9)]">
          {cover ? (
            <Image
              alt={entry.album.name}
              className="object-cover transition-transform duration-500 group-hover:scale-105"
              fill
              sizes="220px"
              src={cover}
            />
          ) : (
            <AlbumArtFallback />
          )}
          <div className="absolute inset-0 bg-black/20 opacity-0 transition-opacity group-hover:opacity-100" />
          <BannedBadge
            className="absolute bottom-2 left-2 rounded-full bg-black/75 p-1"
            target={{ kind: "album", id: entry.album.id }}
          />
        </div>
        <div className="mt-3 min-w-0">
          <h4
            className="line-clamp-2 text-sm font-semibold leading-5 text-zinc-100"
            title={entry.album.name}
          >
            {entry.album.name}
          </h4>
          <p className="mt-0.5 truncate text-[13px] text-zinc-400">
            {[
              entry.artist.id !== personId ? entry.artist.name : null,
              ...performers,
              entry.year,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
            <QualityBadge album={entry} />
            {ownRoles.slice(0, 2).map((role) => (
              <span
                className="rounded-full bg-white/6 px-2 py-px text-[10px] font-semibold uppercase tracking-wide text-zinc-400"
                key={role}
              >
                {focusRoleLabel(role)}
              </span>
            ))}
          </div>
        </div>
      </Link>
    </AlbumMenu>
  );
}

/**
 * A person's whole discography with Focus: their own releases, what they
 * conducted, appear on, composed or produced, narrowed by stackable chips
 * for role, the other people on each album, form, period, resolution and
 * source.
 */
export function ArtistDiscography({
  discography,
  selection,
  onSelectionChange,
  loading,
}: {
  discography: PersonDiscography;
  selection: FocusSelection;
  onSelectionChange: (selection: FocusSelection) => void;
  loading: boolean;
}) {
  const sections = useMemo(
    () => discographySections(discography.albums),
    [discography.albums],
  );
  const focused = discography.focus.selected.length > 0;
  const onlyLocal = !discography.albums.some((entry) =>
    entry.sources.some((source) => source !== "local"),
  );
  return (
    <div className="space-y-10">
      <FocusPanel
        loading={loading}
        note={
          onlyLocal ? (
            <span>
              Source: every album here is a local file. Streaming albums join
              the Source filter once a streaming service adds them to the
              library.
            </span>
          ) : null
        }
        onChange={onSelectionChange}
        selection={selection}
        summary={discography.focus}
      />
      {discography.albums.length === 0 && (
        <div className="rounded-xl border border-dashed border-white/10 px-6 py-10 text-center">
          <p className="text-sm text-zinc-300">
            {focused
              ? "No album matches every filter."
              : `No albums by ${discography.name} are in this library.`}
          </p>
          {focused && (
            <button
              className="mt-3 text-sm font-semibold text-white hover:underline"
              onClick={() => onSelectionChange({})}
              type="button"
            >
              Clear filters
            </button>
          )}
        </div>
      )}
      {sections.map((section) => (
        <section aria-label={section.title} key={section.key}>
          <h3 className="mb-5 flex items-baseline gap-2 text-xl font-semibold text-white">
            {section.title}
            <span className="text-sm font-medium text-zinc-500">
              {section.albums.length}
            </span>
          </h3>
          <div className="grid grid-cols-2 gap-x-5 gap-y-7 sm:grid-cols-3 lg:grid-cols-4">
            {section.albums.map((entry) => (
              <AlbumTile
                entry={entry}
                key={entry.album.id}
                personId={discography.artist_id}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
