"use client";

import { defaultCover } from "@/lib/images/default-cover";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import Image from "next/image";
import Link from "next/link";
import type { StreamingProviderId } from "@parson/music-sdk";
import AlbumMenu from "@/features/library/album-menu";
import { SourceBadge } from "@/features/streaming/source-badge";

type AlbumCardProps = {
  artist_id: string;
  artist_name: string;
  album_id: string;
  album_name: string;
  album_cover: string;
  first_release_date: string;
  typeLabel?: "Album";
  /** Streaming service the album plays from; local albums have none. */
  source?: StreamingProviderId;
  /** Quality badge, e.g. `24/96 FLAC`. */
  formatLabel?: string | null;
};

export default function AlbumCard({
  artist_id,
  artist_name,
  album_id,
  album_name,
  album_cover,
  typeLabel,
  source,
  formatLabel,
}: AlbumCardProps) {
  const albumCoverURL =
    getLibraryImageUrl(album_cover, getBaseURL) ?? defaultCover;
  const card = (
    <div className="group relative w-full min-w-0">
      <Link
        aria-label={`View ${album_name}`}
        className="absolute inset-0 z-0 rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
        href={`/album?id=${encodeURIComponent(album_id)}`}
      />
      <div className="pointer-events-none relative z-1 aspect-square w-full overflow-hidden rounded-lg bg-[#111]">
        <Image
          src={albumCoverURL}
          alt={`${album_name} cover`}
          height={800}
          width={800}
          onError={(event) => {
            event.currentTarget.src = defaultCover;
          }}
          className="h-full w-full object-cover"
        />
        {source && (
          <span className="absolute bottom-2 left-2">
            <SourceBadge provider={source} variant="overlay" />
          </span>
        )}
      </div>

      <div className="pointer-events-none relative z-1 mt-3 w-full">
        <p
          className="line-clamp-2 text-sm font-semibold leading-5 text-zinc-100"
          title={album_name}
        >
          {album_name}
        </p>

        {artist_name && (
          <p className="min-w-0 text-sm leading-5 text-zinc-500">
            {typeLabel && <span>{typeLabel} · </span>}
            <Link
              className="pointer-events-auto relative z-10 truncate hover:text-white hover:underline"
              href={`/artist?id=${encodeURIComponent(artist_id)}`}
            >
              {artist_name}
            </Link>
          </p>
        )}
        {formatLabel && (
          <p className="mt-0.5 text-xs tabular-nums text-zinc-600">
            {formatLabel}
          </p>
        )}
      </div>
    </div>
  );

  return source ? (
    card
  ) : (
    <AlbumMenu album_id={album_id} artist_id={artist_id}>
      {card}
    </AlbumMenu>
  );
}
