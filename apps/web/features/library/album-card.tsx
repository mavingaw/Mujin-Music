"use client";

import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import Image from "next/image";
import Link from "next/link";
import type { StreamingProviderId } from "@parson/music-sdk";
import AlbumMenu from "@/features/library/album-menu";
import { SourceBadge } from "@/features/streaming/source-badge";
import { BannedBadge } from "@/features/library/radio-bans";
import { generatedCover } from "@/features/library/artwork/generated-cover";
import { QualityBadge } from "@/features/library/artwork/quality-badge";
import { usePlayAlbum } from "@/features/library/use-play-album";
import { PlayNowSplit } from "@/features/player/play-now-split";

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
  /** The badge marks high-resolution lossless (copper dot). */
  hiRes?: boolean;
  lossless?: boolean;
  /** Show the release year after the artist. */
  showYear?: boolean;
};

export default function AlbumCard({
  artist_id,
  artist_name,
  album_id,
  album_name,
  album_cover,
  first_release_date,
  typeLabel,
  source,
  formatLabel,
  hiRes = false,
  lossless = false,
  showYear = false,
}: AlbumCardProps) {
  const playAlbum = usePlayAlbum();
  const fallback = generatedCover(album_name, artist_name);
  const albumCoverURL = getLibraryImageUrl(album_cover, getBaseURL) ?? fallback;
  const year = showYear ? first_release_date.slice(0, 4) : "";
  const card = (
    <div className="mj-card group relative w-full min-w-0">
      <Link
        aria-label={`View ${album_name}`}
        className="absolute inset-0 z-0 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-mujin-copper"
        href={`/album?id=${encodeURIComponent(album_id)}`}
      />
      <div className="mj-artwork pointer-events-none relative z-1 aspect-square w-full overflow-hidden bg-mujin-surface-1">
        <Image
          src={albumCoverURL}
          alt={`${album_name} cover`}
          height={800}
          width={800}
          onError={(event) => {
            event.currentTarget.src = fallback;
          }}
          className="h-full w-full object-cover transition-transform duration-500 ease-out motion-safe:group-hover:scale-[1.035]"
        />
        <span
          aria-hidden="true"
          className="absolute inset-0 bg-linear-to-t from-black/45 via-transparent to-transparent opacity-0 transition-opacity duration-200 group-hover:opacity-100"
        />
        {source ? (
          <span className="absolute bottom-2 left-2">
            <SourceBadge provider={source} variant="overlay" />
          </span>
        ) : (
          <>
            <BannedBadge
              className="absolute left-2 top-2 rounded-full bg-black/75 p-1"
              target={{ kind: "album", id: album_id }}
            />
            {formatLabel && (
              <QualityBadge
                className="absolute bottom-2 left-2"
                hiRes={hiRes}
                label={formatLabel}
                lossless={lossless}
              />
            )}
            <PlayNowSplit
              className="mj-card-play absolute bottom-2 right-2"
              onPlayNow={() => void playAlbum(album_id)}
              target={{ kind: "album", id: album_id, name: album_name }}
              variant="card"
            />
          </>
        )}
      </div>
      <div className="pointer-events-none relative z-1 mt-2.5 w-full">
        <p
          className="truncate text-[13.5px] font-medium leading-5 text-mujin-text"
          title={album_name}
        >
          {album_name}
        </p>

        {artist_name && (
          <p className="flex min-w-0 text-[12.5px] leading-5 text-mujin-text-secondary">
            {typeLabel && (
              <span className="shrink-0">{typeLabel}&nbsp;·&nbsp;</span>
            )}
            <Link
              className="pointer-events-auto relative z-10 truncate hover:text-mujin-text hover:underline"
              href={`/artist?id=${encodeURIComponent(artist_id)}`}
            >
              {artist_name}
            </Link>
            {year && (
              <span className="mj-tnum shrink-0 text-mujin-text-tertiary">
                &nbsp;· {year}
              </span>
            )}
          </p>
        )}
      </div>
    </div>
  );

  return source ? (
    card
  ) : (
    <AlbumMenu
      album_id={album_id}
      album_name={album_name}
      artist_id={artist_id}
    >
      {card}
    </AlbumMenu>
  );
}
