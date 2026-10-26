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
import { radioSectionStart, radioStationLabel } from "@parson/music-sdk";
import { RadioThumbs } from "./radio/radio-controls";
import { RadioNotices } from "./radio/radio-notices";
import { useSignalPath } from "./signal-path";
import { SignalPathLight } from "./signal-path-popover";
import { DspPanel, isDspShortcut } from "./dsp-panel";

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
    currentIndex,
    radio,
    signal,
    volumeLevelingMode,
    setVolumeLevelingMode,
    dsp,
    setDsp,
    output,
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
  const effectiveSong = castSong || song;
  const casting = Boolean(cast.session && castItem);
  const effectiveCurrentTime = casting
    ? (cast.session?.position_ms ?? 0) / 1000
    : currentTime;
  const effectiveDuration = casting
    ? (cast.session?.duration_ms || castItem?.duration_ms || 0) / 1000
    : duration;
  const effectivePlaying = casting ? Boolean(cast.session?.playing) : isPlaying;
  const effectiveVolume = casting ? (cast.session?.volume ?? volume) : volume;
  const effectiveMuted = casting ? Boolean(cast.session?.muted) : muted;
  const effectiveCover = castItem?.artwork_url || imageSrc || defaultCover;
  const effectiveTitle = castItem?.title || effectiveSong.name;
  const effectiveArtist =
    castItem?.artist ||
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
  const currentItem = casting ? undefined : queue[currentIndex];
  const currentIsRadioPick =
    currentItem?.origin === "generated" &&
    radioSectionStart(
      queue.map((item) => item.origin),
      currentIndex - 1,
    ) === currentIndex;
  const feedback =
    currentItem && currentItem.song.id === song.id ? (
      <RadioThumbs
        index={currentIndex}
        label={currentItem.song.name || "this song"}
        liked={currentItem.liked}
      />
    ) : null;
  const radioLabel = !currentIsRadioPick
    ? null
    : radio.seed && radio.seed.kind !== "song"
      ? radioStationLabel(radio.seed)
      : `Radio · ${radioStationLabel(radio.seed)}`;
  const togglePlayback = casting
    ? () => cast.send({ command: cast.session?.playing ? "pause" : "play" })
    : togglePlayPause;
  const next = casting ? () => cast.send({ command: "next" }) : playNextSong;
  const previous = casting
    ? () => cast.send({ command: "previous" })
    : playPreviousSong;
  const seek = casting
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
  const changeVolume = casting
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
  const toggleOutputMute = casting
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
  const activePreset =
    audioPresets.find((preset) => preset.id === audioPreset) ?? null;
  const playbackRate = activePreset?.rate ?? 1;
  const signalSongId = castItem?.song_id || song.id;
  const { path: signalPath, loading: signalLoading } = useSignalPath({
    audio: effectiveSong.audio,
    castDeviceName: cast.session?.receiver_name,
    casting,
    muted: effectiveMuted,
    preset: activePreset,
    signal,
    songId: signalSongId,
    volume: effectiveVolume,
  });
  const [dspOpen, setDspOpen] = useState(false);
  const openDsp = useCallback(() => setDspOpen(true), []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!isDspShortcut(event) || event.defaultPrevented) return;
      event.preventDefault();
      setDspOpen((value) => !value);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const signalProps = {
    casting,
    onOpenDsp: casting ? undefined : openDsp,
    levelingMode: volumeLevelingMode,
    loading: signalLoading,
    onLevelingModeChange: setVolumeLevelingMode,
    path: signalPath,
    streamBitrate: signal.streamBitrate,
  };

  usePlayerShortcuts({
    currentTime: effectiveCurrentTime,
    duration: effectiveDuration,
    enabled: Boolean(song.id || castItem),
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

  if (!song.id && !castItem) {
    return null;
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
          readPosition={casting ? undefined : getPlaybackPosition}
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
          onReorderQueue={reorderQueue}
          onSelectQueueItem={(index) => {
            playQueueItem(index);
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
          queue={queue}
          queueOpen={queueOpen}
          signalBadge={<SignalPathLight variant="pill" {...signalProps} />}
          slowedReverb={slowedReverb}
          songId={castItem?.song_id || song.id}
          title={displayTitle}
          explicit={isExplicitSongTitle(effectiveTitle)}
          feedback={feedback}
          radioLabel={radioLabel}
          timedLyrics={timedLyrics}
          volume={effectiveVolume}
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
          currentSongId={song.id}
          onClose={closeQueue}
          onReorder={reorderQueue}
          onSelect={(index) => playQueueItem(index)}
          queue={queue}
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

      <DspPanel
        casting={casting}
        onChange={setDsp}
        onOpenChange={setDspOpen}
        open={dspOpen}
        output={output}
        path={signalPath}
        settings={dsp}
      />

      <PlayerFooter
        onOpenDsp={casting ? undefined : openDsp}
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
          casting || fullscreenOpen ? undefined : getPlaybackPosition
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
        signalLight={<SignalPathLight {...signalProps} />}
        slowedReverb={slowedReverb}
        songId={castItem?.song_id || song.id}
        title={displayTitle}
        explicit={isExplicitSongTitle(effectiveTitle)}
        feedback={currentIsRadioPick ? feedback : null}
        volume={effectiveVolume}
      />
      {!casting && !fullscreenOpen && !queueOpen && <RadioNotices />}
    </>
  );
}
