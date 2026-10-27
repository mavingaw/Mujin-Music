"use client";

import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import SongMenu from "@/features/library/song-menu";
import { SongTitle, ExplicitBadge } from "./song-title";
import { isExplicitSongTitle } from "./song-presentation";
import { ArtistCreditLinks } from "./artist-credit-links";
import type { TrackArtistCreditPart } from "./track-artist-credit";
import getBaseURL from "@/lib/api/server-url";
import { generatedCover } from "@/features/library/artwork/generated-cover";
import PlayIcon from "@/components/icons/play";
import Image from "next/image";
import Link from "next/link";

type SongCardProps = {
  song_name: string;
  song_id: string;
  artist_id: string;
  artist_name: string;
  artistCredits?: TrackArtistCreditPart[];
  album_id: string;
  album_name: string;
  album_cover: string;
  path: string;
  typeLabel?: "Song";
};

export default function SongCard({
  song_name,
  song_id,
  artist_id,
  artist_name,
  artistCredits,
  album_id,
  album_name,
  album_cover,
  typeLabel,
}: SongCardProps) {
  const playback = useZonePlayback();
  const fallback = generatedCover(album_name || song_name, artist_name);
  const imageSrc = album_cover
    ? `${getBaseURL()}/media/images/${encodeURIComponent(album_cover)}`
    : fallback;
  const artist = { id: artist_id, name: artist_name };
  const album = { id: album_id, name: album_name, cover_url: album_cover };

  function handlePlay() {
    const songInfo = { id: song_id, name: song_name, artist: artist_name };
    void playback.playQueue([{ song: songInfo, album, artist }]);
  }

  return (
    <div className="w-full min-w-0">
      <SongMenu
        context="home"
        song_name={song_name}
        song_id={song_id}
        artist_id={artist_id}
        artist_name={artist_name}
        album_id={album_id}
        album_name={album_name}
        album_cover={album_cover}
      >
        <div className="mj-card group min-w-0">
          <div className="mj-artwork relative aspect-square w-full overflow-hidden bg-mujin-surface-1">
            <Image
              src={imageSrc}
              alt={song_name}
              fill
              sizes="(min-width: 1024px) 260px, 42vw"
              className="cursor-pointer object-cover transition-transform duration-500 ease-out motion-safe:group-hover:scale-[1.035]"
              onError={(event) => {
                event.currentTarget.src = fallback;
              }}
              onClick={handlePlay}
            />
            <button
              aria-label={`Play ${song_name}`}
              className="mj-card-play mj-primary absolute bottom-2 right-2 grid h-10 w-10 place-items-center rounded-full shadow-[0_8px_24px_-6px_rgba(0,0,0,0.8)]"
              onClick={handlePlay}
              type="button"
            >
              <PlayIcon className="ml-0.5 h-4 w-4" fill="currentColor" />
            </button>
          </div>

          <div className="mt-2.5 min-w-0">
            <Link
              href={`/album?id=${album_id}`}
              className="block truncate text-[0.8438rem] font-medium leading-5 text-mujin-text hover:underline"
            >
              <SongTitle title={song_name} />
            </Link>
            <p className="truncate text-[0.7812rem] leading-5 text-mujin-text-secondary">
              {typeLabel && <span>{typeLabel}&nbsp;·&nbsp;</span>}
              {isExplicitSongTitle(song_name) && <ExplicitBadge />}
              <ArtistCreditLinks
                credits={
                  artistCredits ?? [{ id: artist_id, name: artist_name }]
                }
              />
            </p>
          </div>
        </div>
      </SongMenu>
    </div>
  );
}
