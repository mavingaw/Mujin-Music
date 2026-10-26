"use client";

import { usePlayer, usePlayerClock } from "@/features/player/player-context";
import { defaultCover } from "@/lib/images/default-cover";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  useCloseLyricsOnNavigation,
  useFullscreenDismiss,
  useLyrics,
  usePlayerShortcuts,
} from "./player-bar-hooks";
import { FullscreenPlayer, LyricsPanel } from "./player-overlays";
import { PlayerFooter } from "./player-footer";
import PlayerQueue, { type QueueEditing } from "./player-queue";
import { timelinePlaybackRate } from "./player-controls-state";
import { useMediaSession } from "./use-media-session";
import { useCastOutput } from "./use-cast-output";
import { getSongInfo } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import {
  displaySongTitle,
  isExplicitSongTitle,
} from "@/features/library/song-presentation";
import { trackArtistCreditParts } from "@/features/library/track-artist-credit";
import {
  isStreamingId,
  radioSectionStart,
  radioStationLabel,
} from "@parson/music-sdk";
import { RadioThumbs } from "./radio/radio-controls";
import { RadioNotices } from "./radio/radio-notices";
import { useSignalPath } from "./signal-path";
import { SignalPathLight } from "./signal-path-popover";
import { DspPanel, isDspShortcut, type DspPanelTarget } from "./dsp-panel";
import { useZoneDsp, zoneDspSupport } from "./zones/use-zone-dsp";
import getBaseURL from "@/lib/api/server-url";
import {
  nextRepeatMode,
  previewQueueEdit,
  zoneSignalPath,
  zoneStatusLabel,
  type ZoneCommand,
} from "@parson/music-sdk";
import { useZones } from "./zones/zones-provider";
import { ZonePickerButton } from "./zones/zone-picker";
import { ZoneQualityLight } from "./zones/zone-signal-path";
import {
  useZoneClock,
  useZoneQueue,
  useZoneSong,
} from "./zones/use-remote-zone";
import SpotifyHandoffDialog, {
  useSpotifyHandoffAvailable,
} from "@/features/imports/spotify-handoff-dialog";

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
    removeQueueItem,
    clearUpcoming,
    repeatMode,
    setRepeatMode,
    shuffle,
    setShuffle,
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
    output,
  } = usePlayer();
  const currentTime = usePlayerClock();
  const handoffAvailable = useSpotifyHandoffAvailable();
  const [handoffOpen, setHandoffOpen] = useState(false);
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
  // Volume goes through the zone when it is remote, or when this device's
  // own zone hands its volume to Home Assistant.
  const volumeZone =
    remote ??
    (zones.localZone?.volume?.control === "home_assistant"
      ? zones.localZone
      : null);
  const remoteSong = useZoneSong(remote);
  // An edit made from here shows at once, until the zone reports it.
  const [optimistic, setOptimistic] = useState<{
    zoneId: string;
    base: string;
    ids: string[];
  } | null>(null);
  const remoteQueueKey = remote ? remote.queue.song_ids.join(",") : "";
  const optimisticIds =
    optimistic &&
    remote &&
    optimistic.zoneId === remote.id &&
    optimistic.base === remoteQueueKey
      ? optimistic.ids
      : null;
  const remoteQueue = useZoneQueue(remote, optimisticIds);
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
    ? // The queued track's own length first (the speaker's lags a change).
      ((remoteNow?.song_id ? remoteNow.duration_ms : 0) ||
        remote.duration_ms ||
        remoteNow?.duration_ms ||
        0) / 1000
    : casting
      ? (cast.session?.duration_ms || castItem?.duration_ms || 0) / 1000
      : duration;
  const effectivePlaying = remote
    ? remote.status === "playing"
    : casting
      ? Boolean(cast.session?.playing)
      : isPlaying;
  const effectiveVolume = volumeZone
    ? (volumeZone.volume?.level ?? 0) / 100
    : casting
      ? (cast.session?.volume ?? volume)
      : volume;
  const effectiveMuted = volumeZone
    ? Boolean(volumeZone.volume?.muted)
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
  const currentItem = casting || remote ? undefined : queue[currentIndex];
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
  const sendRemote = (command: Parameters<typeof zones.send>[1]) => {
    if (!remote) return;
    void zones.send(remote.id, command).catch(() => {});
  };
  const sendQueueEdit = (command: ZoneCommand) => {
    if (!remote) return;
    const ids = optimisticIds ?? remote.queue.song_ids;
    const preview = previewQueueEdit(
      ids,
      remote.queue.current_position,
      command,
    );
    if (!preview) return;
    setOptimistic({ zoneId: remote.id, base: remoteQueueKey, ids: preview });
    void zones.send(remote.id, command).catch(() => setOptimistic(null));
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
  const sendVolume = (command: ZoneCommand) => {
    if (!volumeZone) return;
    void zones.send(volumeZone.id, command).catch(() => {});
  };
  const changeVolume = volumeZone
    ? (value: number | string) => {
        const level = Math.round(Number(value));
        if (Number.isFinite(level)) {
          sendVolume({
            command: "set_volume",
            volume: Math.min(volumeZone.volume?.max ?? 100, Math.max(0, level)),
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
  const toggleOutputMute = volumeZone
    ? () =>
        sendVolume({ command: "set_mute", muted: !volumeZone.volume?.muted })
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
  const activePreset =
    audioPresets.find((preset) => preset.id === audioPreset) ?? null;
  // Presets slow or speed up this device only, never a remote zone's times.
  const playbackRate = timelinePlaybackRate(
    activePreset?.rate ?? 1,
    Boolean(remote) || casting,
  );
  const signalSongId = castItem?.song_id || song.id;
  // A zone the server plays (a device, bridge or network player) carries
  // its whole planned chain; other remote zones show their own summary.
  const remotePath = useMemo(
    () => (remote ? zoneSignalPath(remote) : null),
    [remote],
  );
  // TIDAL and Qobuz tracks show the format their stream delivered instead.
  const hideSignalPath = remote ? !remotePath : isStreamingId(signalSongId);
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
  // The DSP engine edits the chain of the zone the bar controls (or one
  // picked in the zone picker): this browser's own, or a remote zone's,
  // which the server runs before that zone's output.
  const { closeDsp, dspPanel, localDsp, openDsp: openZoneDsp } = zones;
  const dspZone =
    dspPanel.zoneId && dspPanel.zoneId !== zones.localZoneId
      ? (zones.zones.find((zone) => zone.id === dspPanel.zoneId) ?? null)
      : null;
  const remoteDsp = useZoneDsp(dspZone);
  const remoteId = remote?.id ?? null;
  const dspAvailable = remote
    ? Boolean(remote.settings?.supports.dsp)
    : !casting;
  const openDsp = useCallback(
    () => openZoneDsp(remoteId),
    [openZoneDsp, remoteId],
  );
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!isDspShortcut(event) || event.defaultPrevented) return;
      event.preventDefault();
      if (dspPanel.open) closeDsp();
      else openZoneDsp(remoteId);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closeDsp, dspPanel.open, openZoneDsp, remoteId]);
  const dspTarget: DspPanelTarget = dspZone
    ? {
        kind: "server",
        name: dspZone.name,
        supported: Boolean(dspZone.settings?.supports.dsp),
        support: zoneDspSupport(dspZone),
      }
    : casting
      ? { kind: "cast" }
      : { kind: "client", output };
  const signalProps = {
    casting,
    onOpenDsp: dspAvailable ? openDsp : undefined,
    levelingMode: volumeLevelingMode,
    loading: remote ? false : signalLoading,
    onLevelingModeChange: setVolumeLevelingMode,
    path: remote ? remotePath : signalPath,
    streamBitrate: signal.streamBitrate,
    zoneId: remote?.id ?? null,
  };

  usePlayerShortcuts({
    currentTime: effectiveCurrentTime,
    duration: effectiveDuration,
    enabled: Boolean(song.id || castItem || remote),
    onNext: next,
    onPrevious: previous,
    onRepeat: remote
      ? () => {
          if (remote.capabilities.repeat) {
            sendRemote({
              command: "set_repeat",
              mode: nextRepeatMode(remote.repeat),
            });
          }
        }
      : toggleLoop,
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
  const remoteIds = optimisticIds ?? remote?.queue.song_ids ?? [];
  const reorderPlayerQueue = remote
    ? (from: number, to: number) =>
        sendQueueEdit({
          command: "move_queue_item",
          from,
          to,
          song_id: remoteIds[from],
        })
    : reorderQueue;
  const queueSongId = remote ? (remoteNow?.song_id ?? "") : song.id;
  const localIndex = queue.findIndex((item) => item.song.id === song.id);
  const queueEditing = ((): QueueEditing | undefined => {
    if (remote) {
      const parsonQueue = remote.source === "parson";
      return {
        currentIndex: remote.queue.current_position,
        editable: remote.capabilities.queue_edit && parsonQueue,
        onRemove: (index) =>
          sendQueueEdit({
            command: "remove_queue_item",
            queue_position: index,
            song_id: remoteIds[index],
          }),
        onClearUpcoming: () => sendQueueEdit({ command: "clear_upcoming" }),
        repeatMode: remote.repeat,
        onRepeatChange: remote.capabilities.repeat
          ? (mode) => sendRemote({ command: "set_repeat", mode })
          : null,
        shuffle: remote.shuffle,
        onShuffleChange: remote.capabilities.shuffle
          ? (enabled) => sendRemote({ command: "set_shuffle", enabled })
          : null,
      };
    }
    if (casting) return undefined;
    return {
      currentIndex: Math.max(0, localIndex),
      editable: queue.length > 0,
      reorderAnywhere: true,
      onRemove: removeQueueItem,
      onClearUpcoming: clearUpcoming,
      repeatMode,
      onRepeatChange: setRepeatMode,
      shuffle,
      onShuffleChange: setShuffle,
    };
  })();
  const effectiveRepeat = remote ? remote.repeat : repeatMode;
  const cycleRepeat = remote
    ? () =>
        sendRemote({
          command: "set_repeat",
          mode: nextRepeatMode(remote.repeat),
        })
    : toggleLoop;

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
          repeatMode={effectiveRepeat}
          queueEditing={queueEditing}
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
          onToggleLoop={cycleRepeat}
          onToggleMute={toggleOutputMute}
          onTogglePlayback={togglePlayback}
          onToggleSound={toggleSlowedReverb}
          onVolumeChange={changeVolume}
          playbackRate={playbackRate}
          queue={playerQueue}
          queueOpen={queueOpen}
          signalBadge={
            hideSignalPath ? null : (
              <SignalPathLight variant="pill" {...signalProps} />
            )
          }
          slowedReverb={slowedReverb}
          songId={remote ? queueSongId : castItem?.song_id || song.id}
          title={displayTitle}
          explicit={isExplicitSongTitle(effectiveTitle)}
          feedback={feedback}
          radioLabel={radioLabel}
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
          editing={queueEditing}
          title={remote ? `Queue · ${remote.name}` : "Queue"}
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

      <DspPanel
        error={dspZone ? remoteDsp.error : localDsp.error}
        onChange={dspZone ? remoteDsp.setSettings : localDsp.setSettings}
        onOpenChange={(open) => {
          if (!open) closeDsp();
        }}
        open={dspPanel.open}
        path={dspZone ? zoneSignalPath(dspZone) : signalPath}
        settings={dspZone ? remoteDsp.settings : localDsp.settings}
        target={dspTarget}
      />

      <PlayerFooter
        onOpenDsp={dspAvailable ? openDsp : undefined}
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
        onHandOffToSpotify={
          handoffAvailable && !casting ? () => setHandoffOpen(true) : undefined
        }
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
        signalLight={
          hideSignalPath ? null : <SignalPathLight {...signalProps} />
        }
        slowedReverb={slowedReverb}
        songId={remote ? queueSongId : castItem?.song_id || song.id}
        title={displayTitle}
        explicit={isExplicitSongTitle(effectiveTitle)}
        feedback={currentIsRadioPick ? feedback : null}
        volume={effectiveVolume}
        zonePicker={<ZonePickerButton />}
        zoneLabel={remote ? remote.name : null}
        zoneSignal={
          remote && !remotePath ? (
            <ZoneQualityLight compact zone={remote} />
          ) : null
        }
      />
      {!casting && !fullscreenOpen && !queueOpen && <RadioNotices />}
      {handoffOpen && (
        <SpotifyHandoffDialog
          onHandedOff={() => {
            if (isPlaying) togglePlayPause();
          }}
          onOpenChange={setHandoffOpen}
          open
          positionMs={currentTime * 1000}
          songIds={handoffSongIds(queue, song.id)}
        />
      )}
    </>
  );
}

/** The current song and the rest of the queue after it. */
function handoffSongIds(queue: { song: { id: string } }[], currentId: string) {
  const start = queue.findIndex((item) => item.song.id === currentId);
  const upcoming = start >= 0 ? queue.slice(start) : [];
  const ids = upcoming.map((item) => item.song.id);
  return ids.length ? ids : [currentId];
}
