"use client";

import FocusBar from "@/features/library/focus/focus-bar";
import FocusPanel from "@/features/library/focus/focus-panel";
import {
  focusCountLabel,
  focusQueryKey,
  focusedAlbumTracks,
  shuffled,
} from "@/features/library/focus/focus-state";
import SmartPlaylistDialog from "@/features/library/focus/smart-playlist-dialog";
import { LibraryMessage } from "@/features/library/library-view-primitives";
import {
  albumScope,
  clearedFocus,
  getFocusPage,
  isFocused,
  scopedFocus,
  type FocusQuery,
  type LibraryAlbum,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Clock, Pause, Play } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";
import SongMenu from "@/features/library/song-menu";
import {
  displaySongTitle,
  isExplicitSongTitle,
} from "@/features/library/song-presentation";
import { SongTitle, ExplicitBadge } from "@/features/library/song-title";
import { BannedBadge } from "@/features/library/radio-bans";
import { trackArtistCreditParts } from "@/features/library/track-artist-credit";

function formatDuration(duration: number) {
  const minutes = Math.floor(duration / 60);
  const seconds = Math.round(duration % 60);
  return `${minutes}:${seconds < 10 ? "0" : ""}${seconds}`;
}

type AlbumTrack = LibraryAlbum["songs"][number];

const FOCUS_PAGE = 200;
const MAX_FOCUS_TRACKS = 1000;

/** Ids of an album's tracks that a scoped focus matches, in focus order. */
async function focusedTrackIds(query: FocusQuery) {
  const first = await getFocusPage(query, 0, FOCUS_PAGE);
  const ids = first.songs.map((song) => song.id);
  for (
    let offset = ids.length;
    offset < Math.min(first.total, MAX_FOCUS_TRACKS);
    offset += FOCUS_PAGE
  ) {
    const next = await getFocusPage(query, offset, FOCUS_PAGE);
    if (!next.songs.length) break;
    ids.push(...next.songs.map((song) => song.id));
  }
  return { ids, total: first.total, libraryTotal: first.libraryTotal };
}

export function AlbumTrackList({
  activeSongId,
  album,
  artistName,
  isPlaying,
  onPlay,
}: {
  activeSongId?: string;
  album: LibraryAlbum;
  artistName: string;
  isPlaying: boolean;
  /** Plays a track with `tracks` (the visible list) as the queue. */
  onPlay: (track: AlbumTrack, tracks: AlbumTrack[]) => void;
}) {
  const [focus, setFocus] = useState<FocusQuery>(() =>
    scopedFocus("songs", [albumScope(album.id)]),
  );
  const [panelOpen, setPanelOpen] = useState(false);
  const [smartOpen, setSmartOpen] = useState(false);
  useEffect(() => {
    setFocus(scopedFocus("songs", [albumScope(album.id)]));
    setPanelOpen(false);
  }, [album.id]);
  const sorted = Boolean(focus.sort);
  const active = isFocused(focus) || sorted;
  const focused = useQuery({
    queryKey: ["library", "focus", "album-tracks", focusQueryKey(focus)],
    queryFn: () => focusedTrackIds(focus),
    enabled: active,
    placeholderData: keepPreviousData,
  });
  const tracks = focusedAlbumTracks(
    album.songs,
    active ? focused.data?.ids : undefined,
    sorted,
  );
  const countLabel =
    active && focused.data
      ? focusCountLabel(focus, focused.data.total, focused.data.libraryTotal)
      : null;
  const playTracks = (shuffle: boolean) => {
    const queue = shuffle ? shuffled(tracks) : tracks;
    const first = queue[0];
    if (first) onPlay(first, queue);
  };

  return (
    <div className="w-full pb-8">
      {album.songs.length > 1 && (
        <div className="mb-5 space-y-3">
          <FocusBar
            bookmarks={false}
            canPlay={tracks.length > 0}
            onCreateSmartPlaylist={() => setSmartOpen(true)}
            onPlay={playTracks}
            onTogglePanel={() => setPanelOpen((open) => !open)}
            panelOpen={panelOpen}
            query={focus}
            setQuery={setFocus}
          />
          {panelOpen && <FocusPanel query={focus} setQuery={setFocus} />}
          {countLabel && (
            <p
              aria-live="polite"
              className="text-sm tabular-nums text-zinc-500"
            >
              {countLabel}
            </p>
          )}
          <SmartPlaylistDialog
            onOpenChange={setSmartOpen}
            open={smartOpen}
            query={focus}
          />
        </div>
      )}
      {active && focused.data && tracks.length === 0 ? (
        <LibraryMessage
          action="Clear focus"
          body="No track on this album matches every filter."
          onAction={() => setFocus(clearedFocus(focus))}
          title="No matching tracks"
        />
      ) : (
        <TrackRows
          activeSongId={activeSongId}
          album={album}
          artistName={artistName}
          isPlaying={isPlaying}
          onPlay={(track) => onPlay(track, tracks)}
          tracks={tracks}
        />
      )}
    </div>
  );
}

function TrackRows({
  activeSongId,
  album,
  artistName,
  isPlaying,
  onPlay,
  tracks,
}: {
  activeSongId?: string;
  album: LibraryAlbum;
  artistName: string;
  isPlaying: boolean;
  onPlay: (track: AlbumTrack) => void;
  tracks: AlbumTrack[];
}) {
  return (
    <div>
      <div className="mb-2 grid select-none grid-cols-[2rem_minmax(0,1fr)_3rem] gap-4 border-b border-white/10 px-4 py-2 text-sm font-medium text-zinc-500">
        <div className="text-center">#</div>
        <div>Track</div>
        <div className="flex justify-end text-right">
          <Clock className="h-4 w-4" />
        </div>
      </div>
      <div className="flex flex-col space-y-1">
        {tracks.map((track, index) => {
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
          return (
            <SongMenu
              album_id={album.id}
              album_name={album.name}
              album_cover={album.cover_url}
              artist_id={album.artist_object.id}
              artist_name={artistName}
              context="album"
              key={track.id}
              song_id={track.id}
              song_name={track.name}
            >
              <div
                className={`group relative grid min-h-14 grid-cols-[2rem_minmax(0,1fr)_3rem] items-center gap-4 rounded-md px-4 py-3 text-left text-sm transition-colors hover:bg-zinc-800/50 ${
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
                    <BannedBadge
                      className="ml-1.5 align-[-2px]"
                      target={{ kind: "song", id: track.id }}
                    />
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
                </div>
                <div className="pointer-events-none relative z-10 text-right tabular-nums text-zinc-400">
                  {formatDuration(track.duration)}
                </div>
              </div>
            </SongMenu>
          );
        })}
      </div>
    </div>
  );
}
