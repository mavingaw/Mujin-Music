"use client";

import { usePlayer, usePlayerClock } from "@/features/player/player-context";
import { defaultCover } from "@/lib/images/default-cover";
import { useCallback, useEffect, useState } from "react";
import {
  useCloseLyricsOnNavigation,
  useFullscreenDismiss,
  useLyrics,
  usePlayerShortcuts,
} from "./player-bar-hooks";
import { FullscreenPlayer, LyricsPanel } from "./player-overlays";
import { PlayerFooter } from "./player-footer";
import PlayerQueue from "./player-queue";
import { useMediaSession } from "./use-media-session";
import { useCastOutput } from "./use-cast-output";
import { getSongInfo } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import {
  displaySongTitle,
  isExplicitSongTitle,
} from "@/features/library/song-presentation";
import { trackArtistCreditParts } from "@/features/library/track-artist-credit";
import getBaseURL from "@/lib/api/server-url";
import { zoneStatusLabel } from "@parson/music-sdk";
import { useZones } from "./zones/zones-provider";
import { ZonePickerButton } from "./zones/zone-picker";
import {
  useZoneClock,
  useZoneQueue,
  useZoneSong,
} from "./zones/use-remote-zone";

export default function PlayerBar() {
  const {
    album,
    artist,
    duration,
    error,
    handleTimeChange,
    getPlaybackPosition,
    imageSrc,
    isPlaying,
    looping,
    muted,
    playNextSong,
    playPreviousSong,
    playQueueItem,
    queue,
    reorderQueue,
    setAudioVolume,
    setAudioPreset,
    slowedReverb,
    song,
    toggleLoop,
    toggleMute,
    togglePlayPause,
    toggleSlowedReverb,
    volume,
    audioPreset,
    audioPresets,
  } = usePlayer();
  const currentTime = usePlayerClock();
  const cast = useCastOutput();
  const castItem = cast.session?.items[cast.session.current_position] ?? null;
  const castSong = useQuery({
    queryKey: ["cast-song", castItem?.song_id],
    queryFn: () => getSongInfo(castItem!.song_id, false),
    enabled: Boolean(castItem?.song_id),
    staleTime: 5 * 60_000,
  }).data;
  // A remote zone (another device, Sonos, a group) takes over the bar.
  const zones = useZones();
  const remote = zones.remoteZone;
  const remoteSong = useZoneSong(remote);
  const remoteQueue = useZoneQueue(remote);
  const remoteTime = useZoneClock(remote, zones.fetchedAt);
  const remoteNow = remote?.now_playing ?? null;
  const effectiveSong = (remote ? remoteSong : castSong) || song;
  const casting = !remote && Boolean(cast.session && castItem);
  const effectiveCurrentTime = remote
    ? remoteTime
    : casting
      ? (cast.session?.position_ms ?? 0) / 1000
      : currentTime;
  const effectiveDuration = remote
    ? (remote.duration_ms || remoteNow?.duration_ms || 0) / 1000
    : casting
      ? (cast.session?.duration_ms || castItem?.duration_ms || 0) / 1000
      : duration;
  const effectivePlaying = remote
    ? remote.status === "playing"
    : casting
      ? Boolean(cast.session?.playing)
      : isPlaying;
  const effectiveVolume = remote
    ? (remote.volume?.level ?? 0) / 100
    : casting
      ? (cast.session?.volume ?? volume)
      : volume;
  const effectiveMuted = remote
    ? Boolean(remote.volume?.muted)
    : casting
      ? Boolean(cast.session?.muted)
      : muted;
  const remoteCover = remoteNow?.cover_url
    ? `${getBaseURL()}/media/images/${encodeURIComponent(remoteNow.cover_url)}`
    : defaultCover;
  const effectiveCover = remote
    ? remoteCover
    : castItem?.artwork_url || imageSrc || defaultCover;
  const effectiveTitle = remote
    ? remoteNow?.title || `Nothing playing in ${remote.name}`
    : castItem?.title || effectiveSong.name;
  const effectiveArtist = remote
    ? remoteNow?.artist ||
      (remoteNow ? "" : zoneStatusLabel(remote)) ||
      effectiveSong.artist
    : castItem?.artist ||
      effectiveSong.artist_object.name ||
      effectiveSong.artist;
  const displayTitle = displaySongTitle(effectiveTitle);
  const artistCredits = trackArtistCreditParts({
    albumArtist: effectiveSong.artist_object.name || effectiveArtist,
    albumArtistId: effectiveSong.artist_object.id || artist.id,
    albumType: effectiveSong.album_object.primary_type,
    contributingArtists: effectiveSong.contributing_artists ?? [],
    contributingArtistIds: effectiveSong.contributing_artist_ids ?? [],
    trackArtist: effectiveSong.artist || effectiveArtist,
  }) ?? [
    { id: effectiveSong.artist_object.id || artist.id, name: effectiveArtist },
  ];
  const displayArtist = artistCredits.map((credit) => credit.name).join(", ");
  const sendRemote = (command: Parameters<typeof zones.send>[1]) => {
    if (!remote) return;
    void zones.send(remote.id, command).catch(() => {});
  };
  const togglePlayback = remote
    ? () => sendRemote({ command: effectivePlaying ? "pause" : "play" })
    : casting
      ? () => cast.send({ command: cast.session?.playing ? "pause" : "play" })
      : togglePlayPause;
  const next = remote
    ? () => sendRemote({ command: "next" })
    : casting
      ? () => cast.send({ command: "next" })
      : playNextSong;
  const previous = remote
    ? () => sendRemote({ command: "previous" })
    : casting
      ? () => cast.send({ command: "previous" })
      : playPreviousSong;
  const seek = remote
    ? (value: number | string) => {
        const seconds = Number(value);
        if (Number.isFinite(seconds)) {
          sendRemote({
            command: "seek",
            position_ms: Math.max(0, Math.round(seconds * 1000)),
          });
        }
      }
    : casting
      ? (value: number | string) => {
          const seconds = Number(value);
          if (Number.isFinite(seconds)) {
            cast.send({
              command: "seek",
              position_ms: Math.max(0, Math.round(seconds * 1000)),
            });
          }
        }
      : handleTimeChange;
  const changeVolume = remote
    ? (value: number | string) => {
        const level = Math.round(Number(value));
        if (Number.isFinite(level)) {
          sendRemote({
            command: "set_volume",
            volume: Math.min(remote.volume?.max ?? 100, Math.max(0, level)),
          });
        }
      }
    : casting
      ? (value: number | string) => {
          const level = Number(value) / 100;
          if (Number.isFinite(level)) {
            cast.send({
              command: "set_volume",
              volume: Math.min(1, Math.max(0, level)),
            });
          }
        }
      : setAudioVolume;
  const toggleOutputMute = remote
    ? () => sendRemote({ command: "set_mute", muted: !remote.volume?.muted })
    : casting
      ? () => cast.send({ command: "set_mute", muted: !cast.session?.muted })
      : toggleMute;
  const [lyricsOpen, setLyricsOpen] = useState(false);
  const [fullscreenOpen, setFullscreenOpen] = useState(false);
  const [queueOpen, setQueueOpen] = useState(false);
  const closeLyrics = useCallback(() => setLyricsOpen(false), []);
  const closeFullscreen = useCallback(() => {
    setFullscreenOpen(false);
    setLyricsOpen(false);
    setQueueOpen(false);
  }, []);
  const closeQueue = useCallback(() => setQueueOpen(false), []);
  const cover = effectiveCover;
  const playbackRate =
    audioPresets.find((preset) => preset.id === audioPreset)?.rate ?? 1;

  usePlayerShortcuts({
    currentTime: effectiveCurrentTime,
    duration: effectiveDuration,
    enabled: Boolean(song.id || castItem || remote),
    onNext: next,
    onPrevious: previous,
    onRepeat: toggleLoop,
    onSeek: seek,
    onToggleMute: toggleOutputMute,
    onTogglePlayback: togglePlayback,
  });
  useMediaSession({
    album: effectiveSong.album_object.name || album.name,
    artist: effectiveArtist,
    artwork: cover,
    currentTime: effectiveCurrentTime,
    duration: effectiveDuration,
    isPlaying: effectivePlaying,
    onNext: next,
    onPause: togglePlayback,
    onPlay: togglePlayback,
    onPrevious: previous,
    onSeek: seek,
    title: effectiveTitle,
  });
  const {
    activeLine,
    activeLineRef,
    fallback: fallbackLyrics,
    failed: lyricsFailed,
    instrumental: lyricsInstrumental,
    loading: lyricsLoading,
    retry: retryLyrics,
    scrollRef: lyricsScrollRef,
    timed: timedLyrics,
  } = useLyrics(effectiveSong, effectiveCurrentTime, lyricsOpen);
  useFullscreenDismiss(fullscreenOpen, closeFullscreen);
  useEffect(() => {
    if (!lyricsOpen || fullscreenOpen) {
      return;
    }
    const onEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) {
        return;
      }
      event.preventDefault();
      closeLyrics();
      document
        .querySelector<HTMLButtonElement>('footer button[aria-label="Lyrics"]')
        ?.focus();
    };
    window.addEventListener("keydown", onEscape);
    return () => window.removeEventListener("keydown", onEscape);
  }, [closeLyrics, fullscreenOpen, lyricsOpen]);
  useCloseLyricsOnNavigation(closeLyrics);

  const playerQueue = remote ? remoteQueue : queue;
  const selectQueueItem = remote
    ? (index: number) => sendRemote({ command: "jump", queue_position: index })
    : playQueueItem;
  const reorderPlayerQueue = remote ? () => {} : reorderQueue;
  const queueSongId = remote ? (remoteNow?.song_id ?? "") : song.id;

  if (!song.id && !castItem && !remote) {
    // Nothing plays here: keep the zone picker reachable so a remote zone
    // can be chosen and controlled.
    return zones.zones.length > 1 ? (
      <div className="fixed bottom-22 right-3 z-50 rounded-full border border-white/10 bg-[#080808] p-1 shadow-2xl md:bottom-5 md:right-5">
        <ZonePickerButton />
      </div>
    ) : null;
  }

  return (
    <>
      {fullscreenOpen && (
        <FullscreenPlayer
          audioPreset={audioPreset}
          audioPresets={audioPresets}
          onSelectPreset={setAudioPreset}
          activeLine={activeLine}
          activeLineRef={activeLineRef}
          artistCredits={artistCredits}
          cover={cover}
          currentTime={effectiveCurrentTime}
          duration={effectiveDuration}
          readPosition={casting || remote ? undefined : getPlaybackPosition}
          isPlaying={effectivePlaying}
          looping={looping}
          lyricsOpen={lyricsOpen}
          lyricsFallback={fallbackLyrics}
          lyricsFailed={lyricsFailed}
          lyricsInstrumental={lyricsInstrumental}
          lyricsLoading={lyricsLoading}
          lyricsScrollRef={lyricsScrollRef}
          muted={effectiveMuted}
          onClose={closeFullscreen}
          onNext={next}
          onOpenLyrics={() => {
            setQueueOpen(false);
            setLyricsOpen((open) => !open);
          }}
          onOpenQueue={() => {
            setLyricsOpen(false);
            setQueueOpen(true);
          }}
          onBackToPlayer={() => {
            setLyricsOpen(false);
            setQueueOpen(false);
          }}
          onPrevious={previous}
          onRetryLyrics={retryLyrics}
          onReorderQueue={reorderPlayerQueue}
          onSelectQueueItem={(index) => {
            selectQueueItem(index);
            setLyricsOpen(false);
            setQueueOpen(false);
          }}
          onSeek={seek}
          onToggleLoop={toggleLoop}
          onToggleMute={toggleOutputMute}
          onTogglePlayback={togglePlayback}
          onToggleSound={toggleSlowedReverb}
          onVolumeChange={changeVolume}
          playbackRate={playbackRate}
          queue={playerQueue}
          queueOpen={queueOpen}
          slowedReverb={slowedReverb}
          songId={remote ? queueSongId : castItem?.song_id || song.id}
          title={displayTitle}
          explicit={isExplicitSongTitle(effectiveTitle)}
          timedLyrics={timedLyrics}
          volume={effectiveVolume}
          zonePicker={<ZonePickerButton compact />}
        />
      )}

      {lyricsOpen && !fullscreenOpen && (
        <LyricsPanel
          activeLine={activeLine}
          activeLineRef={activeLineRef}
          cover={cover}
          fallback={fallbackLyrics}
          failed={lyricsFailed}
          instrumental={lyricsInstrumental}
          loading={lyricsLoading}
          onRetry={retryLyrics}
          scrollRef={lyricsScrollRef}
          onSeek={handleTimeChange}
          timed={timedLyrics}
        />
      )}

      {queueOpen && !fullscreenOpen && (
        <PlayerQueue
          currentSongId={queueSongId}
          onClose={closeQueue}
          onReorder={reorderPlayerQueue}
          onSelect={(index) => selectQueueItem(index)}
          queue={playerQueue}
        />
      )}

      {error?.startsWith("This audio format") ? (
        <div
          className="fixed bottom-42 left-1/2 z-80 w-[min(92vw,520px)] -translate-x-1/2 rounded-xl border border-amber-300/20 bg-zinc-950 p-4 shadow-2xl md:bottom-32 md:left-[calc(50%+40px)]"
          role="alert"
        >
          <p className="font-semibold text-white">
            This audio format isn’t supported
          </p>
          <p className="mt-1 text-sm leading-5 text-zinc-400">
            This device can’t decode this track. Try another track or convert
            the file to a supported audio format.
          </p>
        </div>
      ) : null}

      <PlayerFooter
        albumCover={effectiveSong.album_object.cover_url}
        albumId={effectiveSong.album_object.id || album.id}
        albumName={effectiveSong.album_object.name || album.name}
        artistId={effectiveSong.artist_object.id || artist.id}
        artistName={displayArtist}
        artistCredits={artistCredits}
        audioPreset={audioPreset}
        audioPresets={audioPresets}
        cover={cover}
        currentTime={effectiveCurrentTime}
        duration={effectiveDuration}
        readPosition={
          casting || remote || fullscreenOpen ? undefined : getPlaybackPosition
        }
        isPlaying={effectivePlaying}
        lyricsOpen={lyricsOpen}
        muted={effectiveMuted}
        onNext={next}
        onOpenFullscreen={() => {
          setLyricsOpen(false);
          setQueueOpen(false);
          setFullscreenOpen(true);
        }}
        onPrevious={previous}
        onOpenQueue={() => setQueueOpen((open) => !open)}
        onSeek={seek}
        onSelectPreset={setAudioPreset}
        onToggleLyrics={() => setLyricsOpen((open) => !open)}
        onToggleMute={toggleOutputMute}
        onTogglePlayback={togglePlayback}
        onToggleSound={toggleSlowedReverb}
        onVolumeChange={changeVolume}
        playbackRate={playbackRate}
        queueOpen={queueOpen}
        slowedReverb={slowedReverb}
        songId={remote ? queueSongId : castItem?.song_id || song.id}
        title={displayTitle}
        explicit={isExplicitSongTitle(effectiveTitle)}
        volume={effectiveVolume}
        zonePicker={<ZonePickerButton />}
        zoneLabel={remote ? remote.name : null}
      />
    </>
  );
}
