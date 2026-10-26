"use client";

import getBaseURL from "@/lib/api/server-url";
import { AlbumArtFallback, getLibraryImageUrl } from "@/lib/images/image-url";
import type { ArtistCredits, CreditRoleSummary } from "@parson/music-sdk/types";
import Image from "next/image";
import Link from "next/link";
import { useMemo } from "react";
import {
  creditRoleLabel,
  creditRolesLabel,
} from "@/features/library/credits/credit-roles";
import { SongTitle } from "@/features/library/song-title";

export function roleFilterKey(role: {
  role: string;
  instrument: string | null;
}) {
  return role.instrument ? `${role.role}:${role.instrument}` : role.role;
}

function matchesFilter(
  roles: readonly { role: string; instrument: string | null }[],
  filter: string | null,
) {
  if (!filter) return true;
  return roles.some(
    (role) => roleFilterKey(role) === filter || role.role === filter,
  );
}

export function roleSummaryLabel(summary: CreditRoleSummary) {
  const label = creditRoleLabel(summary.role, summary.instrument);
  const tracks = `${summary.track_count} ${summary.track_count === 1 ? "track" : "tracks"}`;
  return { label, detail: tracks };
}

/**
 * Linked credit browsing: everything a person composed, produced or
 * played on, filterable by role.
 */
export function ArtistCreditsPanel({
  credits,
  filter,
  onFilterChange,
}: {
  credits: ArtistCredits;
  filter: string | null;
  onFilterChange: (filter: string | null) => void;
}) {
  const albums = useMemo(
    () => credits.albums.filter((entry) => matchesFilter(entry.roles, filter)),
    [credits.albums, filter],
  );
  const songs = useMemo(
    () => credits.songs.filter((entry) => matchesFilter(entry.roles, filter)),
    [credits.songs, filter],
  );
  const chips = [
    { key: null, label: "All", detail: `${credits.albums.length}` },
    ...credits.roles.map((summary) => ({
      key: roleFilterKey(summary),
      ...roleSummaryLabel(summary),
    })),
  ];

  return (
    <div className="space-y-10">
      <div aria-label="Filter credits by role" className="flex flex-wrap gap-2">
        {chips.map((chip) => {
          const active =
            filter === chip.key ||
            (chip.key !== null &&
              filter !== null &&
              !filter.includes(":") &&
              chip.key === filter);
          return (
            <button
              aria-pressed={active}
              className={`rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors ${
                active
                  ? "bg-white text-black"
                  : "bg-white/6 text-zinc-300 hover:bg-white/10 hover:text-white"
              }`}
              key={chip.key ?? "all"}
              onClick={() => onFilterChange(chip.key)}
              type="button"
            >
              {chip.label}
              <span
                className={`ml-2 text-xs ${active ? "text-black/60" : "text-zinc-500"}`}
              >
                {chip.detail}
              </span>
            </button>
          );
        })}
      </div>

      {albums.length > 0 && (
        <section>
          <h3 className="mb-5 text-xl font-semibold text-white">Albums</h3>
          <div className="grid grid-cols-2 gap-5 sm:grid-cols-3 lg:grid-cols-4">
            {albums.map((entry) => {
              const cover = getLibraryImageUrl(
                entry.album.cover_url,
                getBaseURL,
              );
              const year = /^\d{4}/.exec(
                entry.album.first_release_date ?? "",
              )?.[0];
              return (
                <Link
                  className="group min-w-0"
                  href={`/album?id=${entry.album.id}`}
                  key={entry.album.id}
                >
                  <div className="relative aspect-square w-full overflow-hidden rounded-lg border border-white/10 bg-white/4">
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
                  </div>
                  <h4
                    className="mt-3 line-clamp-2 text-sm font-semibold leading-5 text-zinc-100"
                    title={entry.album.name}
                  >
                    {entry.album.name}
                  </h4>
                  <p className="mt-0.5 truncate text-sm text-zinc-400">
                    {entry.artist.name}
                    {year && ` · ${year}`}
                  </p>
                  <p className="mt-1 truncate text-xs font-medium text-zinc-500">
                    {creditRolesLabel(entry.roles)}
                    {entry.track_ids.length > 0 &&
                      ` · ${entry.track_ids.length} ${entry.track_ids.length === 1 ? "track" : "tracks"}`}
                  </p>
                </Link>
              );
            })}
          </div>
        </section>
      )}

      {songs.length > 0 && (
        <section>
          <h3 className="mb-4 text-xl font-semibold text-white">Tracks</h3>
          <div className="divide-y divide-white/6">
            {songs.map(({ song, roles }) => (
              <Link
                className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 rounded-md px-2 py-3 transition-colors hover:bg-white/4"
                href={`/album?id=${song.album_object?.id ?? song.album_id}`}
                key={song.id}
              >
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium text-white">
                    <SongTitle title={song.name} />
                  </p>
                  <p className="truncate text-sm text-zinc-400">
                    {song.artist_object?.name ?? song.artist}
                    {song.album_object?.name && ` · ${song.album_object.name}`}
                  </p>
                </div>
                <span className="text-right text-xs font-medium text-zinc-500">
                  {creditRolesLabel(roles)}
                </span>
              </Link>
            ))}
          </div>
        </section>
      )}

      {!albums.length && !songs.length && (
        <p className="text-sm text-zinc-500">No credits match this role.</p>
      )}
    </div>
  );
}
