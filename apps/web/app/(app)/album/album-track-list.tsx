"use client";

import type { LibraryAlbum } from "@parson/music-sdk";
import type { AboutCredit } from "@parson/music-sdk/types";
import { Clock, Info, Pause, Play } from "lucide-react";
import { useState } from "react";
import { CreditName } from "@/features/library/credits/credits-list";
import { trackCreditLine } from "@/features/library/credits/credit-roles";
import { TrackCreditsDialog } from "@/features/library/credits/track-credits-dialog";
import Link from "next/link";
import SongMenu from "@/features/library/song-menu";
import {
  displaySongTitle,
  isExplicitSongTitle,
} from "@/features/library/song-presentation";
import { SongTitle, ExplicitBadge } from "@/features/library/song-title";
import { trackArtistCreditParts } from "@/features/library/track-artist-credit";

function formatDuration(duration: number) {
  const minutes = Math.floor(duration / 60);
  const seconds = Math.round(duration % 60);
  return `${minutes}:${seconds < 10 ? "0" : ""}${seconds}`;
}

export function AlbumTrackList({
  activeSongId,
  album,
  artistName,
  isPlaying,
  onPlay,
  trackCredits,
}: {
  activeSongId?: string;
  album: LibraryAlbum;
  artistName: string;
  isPlaying: boolean;
  onPlay: (track: LibraryAlbum["songs"][number]) => void;
  trackCredits?: Map<string, AboutCredit[]>;
}) {
  const [creditsFor, setCreditsFor] = useState<
    LibraryAlbum["songs"][number] | null
  >(null);
  return (
    <div className="w-full pb-8">
      <div className="mb-2 grid select-none grid-cols-[2rem_minmax(0,1fr)_5rem] gap-4 border-b border-white/10 px-4 py-2 text-sm font-medium text-zinc-500">
        <div className="text-center">#</div>
        <div>Track</div>
        <div className="flex justify-end text-right">
          <Clock className="h-4 w-4" />
        </div>
      </div>
      <div className="flex flex-col space-y-1">
        {album.songs.map((track, index) => {
          const active = activeSongId === track.id;
          const artistCredit = trackArtistCreditParts({
            albumArtist: artistName,
            albumArtistId: album.artist_object.id,
            albumType: album.primary_type,
            contributingArtists: track.contributing_artists,
            contributingArtistIds: track.contributing_artist_ids,
            trackArtist: track.artist,
          });
          const displayTitle = displaySongTitle(track.name);
          const credits = trackCredits?.get(track.id) ?? [];
          const creditLine = trackCreditLine(credits);
          return (
            <SongMenu
              album_id={album.id}
              album_name={album.name}
              album_cover={album.cover_url}
              artist_id={album.artist_object.id}
              artist_name={artistName}
              context="album"
              key={track.id}
              onViewCredits={() => setCreditsFor(track)}
              song_id={track.id}
              song_name={track.name}
            >
              <div
                className={`group relative grid min-h-14 grid-cols-[2rem_minmax(0,1fr)_5rem] items-center gap-4 rounded-md px-4 py-3 text-left text-sm transition-colors hover:bg-zinc-800/50 ${
                  active ? "bg-white/2" : ""
                }`}
              >
                <button
                  aria-label={`Play ${displayTitle}`}
                  className="absolute inset-0 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-white/30"
                  onClick={() => onPlay(track)}
                  type="button"
                />
                <div className="pointer-events-none relative z-10 select-none text-center text-zinc-400">
                  <span className={active ? "block" : "group-hover:hidden"}>
                    {active ? (
                      <>
                        {isPlaying ? (
                          <Pause
                            aria-hidden="true"
                            className="mx-auto h-4 w-4 fill-white text-white"
                          />
                        ) : (
                          <Play
                            aria-hidden="true"
                            className="mx-auto h-4 w-4 fill-white text-white"
                          />
                        )}
                        <span className="sr-only">
                          {isPlaying ? "Now playing" : "Current track"}
                        </span>
                      </>
                    ) : (
                      track.track_number || index + 1
                    )}
                  </span>
                  {!active && (
                    <Play
                      aria-hidden="true"
                      className="absolute left-1/2 top-1/2 hidden h-4 w-4 -translate-x-1/2 -translate-y-1/2 fill-white text-white group-hover:block"
                    />
                  )}
                </div>
                <div className="pointer-events-none relative z-10 flex min-w-0 flex-col pr-4 text-left">
                  <span className="truncate font-medium text-white">
                    <SongTitle title={track.name} />
                  </span>
                  {(artistCredit || isExplicitSongTitle(track.name)) && (
                    <span className="mt-0.5 truncate text-zinc-400">
                      {isExplicitSongTitle(track.name) && <ExplicitBadge />}
                      {(
                        artistCredit ?? [
                          { id: album.artist_object.id, name: artistName },
                        ]
                      ).map((credit, creditIndex) => (
                        <span
                          key={`${credit.id ?? credit.name}-${creditIndex}`}
                        >
                          {creditIndex > 0 && ", "}
                          {credit.id ? (
                            <Link
                              className="pointer-events-auto relative z-20 hover:text-white hover:underline"
                              href={`/artist?id=${credit.id}`}
                            >
                              {credit.name}
                            </Link>
                          ) : (
                            credit.name
                          )}
                        </span>
                      ))}
                    </span>
                  )}
                  {creditLine && (
                    <span className="mt-0.5 truncate text-xs text-zinc-500">
                      {creditLine.label}{" "}
                      {creditLine.people.map((person, personIndex) => (
                        <span
                          key={`${person.id ?? person.name}-${personIndex}`}
                        >
                          {personIndex > 0 && ", "}
                          <CreditName
                            className="pointer-events-auto relative z-20 text-zinc-400"
                            credit={{
                              ...person,
                              role:
                                creditLine.label === "Lyrics by"
                                  ? "lyricist"
                                  : "composer",
                            }}
                          />
                        </span>
                      ))}
                    </span>
                  )}
                </div>
                <div className="relative z-10 flex items-center justify-end gap-2 text-right tabular-nums text-zinc-400">
                  <button
                    aria-label={`Credits for ${displayTitle}`}
                    className="rounded p-1 text-zinc-500 opacity-0 transition-opacity hover:text-white focus-visible:opacity-100 group-hover:opacity-100"
                    onClick={() => setCreditsFor(track)}
                    title="View credits"
                    type="button"
                  >
                    <Info className="h-4 w-4" />
                  </button>
                  <span className="pointer-events-none">
                    {formatDuration(track.duration)}
                  </span>
                </div>
              </div>
            </SongMenu>
          );
        })}
      </div>
      {creditsFor && (
        <TrackCreditsDialog
          artistName={creditsFor.artist || artistName}
          audio={creditsFor.audio}
          credits={trackCredits?.get(creditsFor.id) ?? []}
          isrc={creditsFor.isrc}
          label={creditsFor.label}
          onOpenChange={(open) => {
            if (!open) setCreditsFor(null);
          }}
          open
          title={displaySongTitle(creditsFor.name)}
        />
      )}
    </div>
  );
}
