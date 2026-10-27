"use client";

import FavoriteButton from "@/features/library/favorite-button";
import { DeliveredFormat } from "@/features/streaming/delivered-format";
import { NowPlayingStreamingSource } from "@/features/streaming/now-playing-source";
import {
  isLiveRadioId,
  isStreamingId,
  liveStationIdFromSongId,
  type LiveNowPlaying,
} from "@parson/music-sdk";
import {
  LiveStationSave,
  LiveTimeline,
} from "@/features/live-radio/live-player";
import { LiveBadge, StationArtwork } from "@/features/live-radio/station-ui";
import { stationHref } from "@/features/live-radio/live-radio-model";
import { ExplicitBadge } from "@/features/library/song-title";
import { ArtistCreditLinks } from "@/features/library/artist-credit-links";
import type { TrackArtistCreditPart } from "@/features/library/track-artist-credit";
import SongMenu from "@/features/library/song-menu";
import {
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import {
  AudioLines,
  ListMusic,
  Maximize2,
  MicVocal,
  MonitorSpeaker,
  Play,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
} from "lucide-react";
import Image from "next/image";
import { defaultCover } from "@/lib/images/default-cover";
import Link from "next/link";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { AudioPreset, AudioPresetId } from "./audio-presets";
import PlayerMoreMenu from "./player-more-menu";
import { DSP_SHORTCUT_LABEL } from "./dsp-panel";
import {
  ActionButton,
  PlaybackControls,
  Timeline,
  VolumeControl,
} from "./player-controls";

export function PlayerFooter({
  albumCover,
  albumId,
  albumName,
  artistId,
  artistName,
  artistCredits,
  audioPreset,
  audioPresets,
  cover,
  currentTime,
  duration,
  readPosition,
  isPlaying,
  lyricsOpen,
  muted,
  onNext,
  onHandOffToSpotify,
  onOpenFullscreen,
  onOpenDsp,
  onOpenQueue,
  onPrevious,
  onSeek,
  onSelectPreset,
  onToggleLyrics,
  onToggleMute,
  onTogglePlayback,
  onToggleSound,
  onVolumeChange,
  playbackRate,
  queueOpen,
  signalLight,
  slowedReverb,
  songId,
  title,
  explicit,
  feedback,
  volume,
  zonePicker,
  zoneLabel,
  zoneSignal,
  livePlaying,
}: {
  albumCover?: string;
  albumId: string;
  albumName: string;
  artistId: string;
  artistName: string;
  artistCredits: TrackArtistCreditPart[];
  audioPreset: AudioPresetId;
  audioPresets: readonly AudioPreset[];
  cover: string;
  currentTime: number;
  duration: number;
  readPosition?: () => number;
  isPlaying: boolean;
  lyricsOpen: boolean;
  muted: boolean;
  onNext: () => void;
  /** Present when Spotify Connect handoff is available. */
  onHandOffToSpotify?: () => void;
  onOpenFullscreen: () => void;
  /** Opens the DSP engine; omitted while casting. */
  onOpenDsp?: () => void;
  onOpenQueue: () => void;
  onPrevious: () => void;
  onSeek: (value: number) => void;
  onSelectPreset: (id: AudioPresetId) => void;
  onToggleLyrics: () => void;
  onToggleMute: () => void;
  onTogglePlayback: () => void;
  onToggleSound: () => void;
  onVolumeChange: (value: number) => void;
  playbackRate: number;
  queueOpen: boolean;
  /** The signal path quality light, shown beside the transport. */
  signalLight?: ReactNode;
  slowedReverb: boolean;
  songId: string;
  title: string;
  explicit: boolean;
  /** Radio thumbs for the current track, when it is a Radio pick. */
  feedback?: ReactNode;
  volume: number;
  /** Roon-style zone picker, shown next to volume. */
  zonePicker?: ReactNode;
  /** Name of the remote zone being controlled, if any. */
  zoneLabel?: string | null;
  /** The remote zone's quality light, beside "Playing on". */
  zoneSignal?: ReactNode;
  /** What a live station is playing (titles and format), when one plays. */
  livePlaying?: LiveNowPlaying | null;
}) {
  const live = isLiveRadioId(songId);
  const footer = (
    <footer className="fixed bottom-[calc(var(--shell-footer)+8px)] left-2 right-2 z-50 isolate overflow-hidden rounded-xl border border-white/[0.07] bg-mujin-surface-1/94 px-2.5 py-2 text-mujin-text shadow-[0_18px_50px_-12px_rgba(0,0,0,0.85)] backdrop-blur-xl md:inset-x-0 md:bottom-0 md:h-(--shell-footer) md:overflow-visible md:rounded-none md:border-x-0 md:border-b-0 md:border-t md:border-white/[0.06] md:bg-mujin-raised/96 md:px-5 md:py-0 md:shadow-[0_-24px_60px_-30px_rgba(0,0,0,0.9)]">
      <button
        aria-label={`Open now playing: ${title}`}
        className="absolute inset-0 z-20 rounded-xl md:hidden"
        onClick={onOpenFullscreen}
        type="button"
      />
      <div className="relative z-10 grid h-full grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 md:grid-cols-[minmax(0,1fr)_minmax(300px,1.15fr)_minmax(0,1fr)] lg:gap-x-6">
        <TrackInfo
          albumId={albumId}
          artistCredits={artistCredits}
          cover={cover}
          songId={songId}
          title={title}
          explicit={explicit}
          feedback={feedback}
          zoneLabel={zoneLabel ?? null}
          zoneSignal={zoneSignal}
        />
        <section className="relative z-30 flex min-w-0 flex-col items-center gap-1 md:w-full md:max-w-140 md:justify-self-center">
          <div className="flex items-center gap-2">
            {signalLight ? (
              <span className="hidden w-9 justify-center md:flex">
                {signalLight}
              </span>
            ) : null}
            <PlaybackControls
              compact
              prominent
              isPlaying={isPlaying}
              onNext={onNext}
              onPrevious={onPrevious}
              onToggle={onTogglePlayback}
            />
            {signalLight ? (
              <span aria-hidden="true" className="hidden w-9 md:block" />
            ) : null}
          </div>
          {live ? (
            <LiveTimeline
              className="hidden w-full md:flex"
              currentTime={currentTime}
              isPlaying={isPlaying}
              playing={livePlaying}
            />
          ) : (
            <Timeline
              className="hidden w-full md:flex"
              currentTime={currentTime}
              duration={duration}
              isPlaying={isPlaying}
              readPosition={readPosition}
              onChange={onSeek}
              playbackRate={playbackRate}
            />
          )}
        </section>
        <section className="relative z-30 flex items-center justify-end gap-1 lg:justify-self-end">
          <div className={live ? "hidden" : "hidden lg:block"}>
            <ActionButton
              active={lyricsOpen}
              label="Lyrics"
              onClick={onToggleLyrics}
            >
              <MicVocal className="h-4 w-4" />
            </ActionButton>
          </div>
          <span className="hidden md:inline" data-player-queue-trigger>
            <ActionButton
              active={queueOpen}
              label="Queue"
              onClick={onOpenQueue}
              shortcut="Ctrl E"
            >
              <ListMusic className="h-4 w-4" />
            </ActionButton>
          </span>
          <div className="hidden xl:block">
            <ActionButton
              active={slowedReverb}
              label="Sound"
              onClick={onToggleSound}
            >
              <AudioLines className="h-4 w-4" />
            </ActionButton>
          </div>
          {zonePicker && (
            <div
              className="relative z-30 md:ml-1 md:border-l md:border-white/[0.07] md:pl-2"
              data-zone-picker-slot
            >
              {zonePicker}
            </div>
          )}
          <div className="hidden sm:block">
            <VolumeControl
              expanded
              muted={muted}
              onChange={onVolumeChange}
              onToggleMute={onToggleMute}
              volume={volume}
            />
          </div>
          <PlayerMoreMenu>
            <DropdownMenuLabel>Sound</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={audioPreset}
              onValueChange={(value) => onSelectPreset(value as AudioPresetId)}
            >
              {audioPresets.map((preset) => (
                <DropdownMenuRadioItem key={preset.id} value={preset.id}>
                  {preset.label}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator />
            {onOpenDsp ? (
              <DropdownMenuItem onSelect={onOpenDsp}>
                <SlidersHorizontal className="h-4 w-4" />
                DSP engine
                <span className="ml-auto text-[11px] text-zinc-500">
                  {DSP_SHORTCUT_LABEL}
                </span>
              </DropdownMenuItem>
            ) : null}
            {!live && (
              <DropdownMenuItem className="lg:hidden" onSelect={onToggleLyrics}>
                <MicVocal className="h-4 w-4" />
                Lyrics
              </DropdownMenuItem>
            )}
            <DropdownMenuItem onSelect={onOpenFullscreen}>
              <Maximize2 className="h-4 w-4" />
              Fullscreen
            </DropdownMenuItem>
            {onHandOffToSpotify && (
              <DropdownMenuItem onSelect={onHandOffToSpotify}>
                <MonitorSpeaker className="h-4 w-4" />
                Play on Spotify device…
              </DropdownMenuItem>
            )}
          </PlayerMoreMenu>
          <span className="hidden md:inline">
            <ActionButton
              active={false}
              label="Now playing"
              onClick={onOpenFullscreen}
            >
              <Maximize2 className="h-4 w-4" />
            </ActionButton>
          </span>
        </section>
      </div>
      {!live && duration > 0 && (
        <MiniProgress currentTime={currentTime} duration={duration} />
      )}
    </footer>
  );
  // A live station is not a library song: no song menu.
  if (live) return footer;
  return (
    <SongMenu
      album_cover={albumCover}
      album_id={albumId}
      album_name={albumName}
      artist_id={artistId}
      artist_name={artistName}
      context="player"
      song_id={songId}
      song_name={title}
    >
      {footer}
    </SongMenu>
  );
}

/**
 * The docked footer when nothing plays (Roon keeps its footer too): the
 * zone button stays reachable, the transport waits. Desktop only; phones
 * show the zone button on its own when there is more than one zone.
 */
export function IdleFooter({ zonePicker }: { zonePicker?: ReactNode }) {
  return (
    <footer className="fixed inset-x-0 bottom-0 z-50 hidden h-(--shell-footer) items-center border-t border-white/[0.06] bg-mujin-raised/96 px-5 text-mujin-text md:flex">
      <div className="grid w-full grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-6">
        <div className="flex min-w-0 items-center gap-3">
          <span
            aria-hidden="true"
            className="grid h-14 w-14 shrink-0 place-items-center rounded-[5px] bg-mujin-surface-1 shadow-[inset_0_0_0_1px_rgba(242,238,230,0.06)]"
          >
            <span className="mj-signal-dot opacity-40" />
          </span>
          <div className="min-w-0">
            <p className="truncate text-[14px] font-medium text-mujin-text-secondary">
              Nothing playing
            </p>
            <p className="truncate text-[12px] text-mujin-text-tertiary">
              Pick an album to start listening
            </p>
          </div>
        </div>
        <div aria-hidden="true" className="flex items-center gap-2 opacity-35">
          <span className="grid h-10 w-10 place-items-center">
            <SkipBack className="h-5 w-5" />
          </span>
          <span className="grid h-11 w-11 place-items-center rounded-full bg-mujin-text/15">
            <Play className="ml-0.5 h-5 w-5" fill="currentColor" />
          </span>
          <span className="grid h-10 w-10 place-items-center">
            <SkipForward className="h-5 w-5" />
          </span>
        </div>
        <div className="flex items-center justify-end">{zonePicker}</div>
      </div>
    </footer>
  );
}

/** A hairline of progress along the bottom of the phone mini player. */
function MiniProgress({
  currentTime,
  duration,
}: {
  currentTime: number;
  duration: number;
}) {
  const ratio = Math.min(1, Math.max(0, currentTime / (duration || 1)));
  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute inset-x-3 bottom-0 h-[2px] overflow-hidden rounded-full bg-white/10 md:hidden"
    >
      <span
        className="block h-full origin-left bg-mujin-text/85"
        style={{ transform: `scaleX(${ratio})` }}
      />
    </span>
  );
}

function TrackInfo({
  albumId,
  artistCredits,
  cover,
  title,
  explicit,
  feedback,
  songId,
  zoneLabel,
  zoneSignal,
}: {
  albumId: string;
  artistCredits: TrackArtistCreditPart[];
  cover: string;
  title: string;
  explicit: boolean;
  feedback?: ReactNode;
  songId: string;
  zoneLabel: string | null;
  zoneSignal?: ReactNode;
}) {
  const liveStation = liveStationIdFromSongId(songId);
  const href = liveStation
    ? stationHref(liveStation)
    : `/album?id=${encodeURIComponent(albumId)}`;
  return (
    <section className="flex w-fit min-w-0 max-w-full items-center gap-2.5 overflow-hidden md:justify-self-start md:gap-3">
      <Link
        href={href}
        aria-label={`View ${title} album`}
        className="mj-artwork relative h-11 w-11 shrink-0 overflow-hidden rounded-[5px] md:h-14 md:w-14"
      >
        {liveStation ? (
          <StationArtwork
            className="h-full w-full"
            rounded="rounded-[5px]"
            sizes="56px"
            station={{
              name: title,
              logo_url: cover === defaultCover ? null : cover,
            }}
          />
        ) : (
          <Image
            alt={title}
            className="object-cover"
            fill
            sizes="44px"
            src={cover}
          />
        )}
      </Link>
      <div className="min-w-0 flex-[0_1_auto] sm:ml-1">
        <MarqueeTitle href={href} title={title} />
        <span className="flex min-w-0 items-center gap-2 text-[12.5px] text-mujin-text-secondary">
          {liveStation && <LiveBadge className="shrink-0" />}
          <span className="block min-w-0 truncate">
            {explicit && <ExplicitBadge />}
            <ArtistCreditLinks credits={artistCredits} />
          </span>
          {isStreamingId(songId) && (
            <DeliveredFormat
              className="hidden sm:inline-flex"
              songId={songId}
            />
          )}
        </span>
        {zoneLabel && (
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-[11px] font-medium text-mujin-copper">
              Playing on {zoneLabel}
            </span>
            {zoneSignal}
          </span>
        )}
      </div>
      {songId &&
        (liveStation ? (
          <LiveStationSave songId={songId} />
        ) : isStreamingId(songId) ? (
          <NowPlayingStreamingSource songId={songId} title={title} />
        ) : (
          <FavoriteButton songId={songId} songName={title} />
        ))}
      {feedback && (
        <span className="relative z-30 hidden md:flex">{feedback}</span>
      )}
    </section>
  );
}

function MarqueeTitle({ href, title }: { href: string; title: string }) {
  const containerRef = useRef<HTMLAnchorElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);
  const [overflowing, setOverflowing] = useState(false);
  const [hovered, setHovered] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    const text = textRef.current;
    if (!container || !text) {
      return;
    }

    const update = () =>
      setOverflowing(text.scrollWidth > container.clientWidth + 1);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(container);
    observer.observe(text);
    return () => observer.disconnect();
  }, [title]);

  return (
    <Link
      className="relative block w-full max-w-full overflow-hidden whitespace-nowrap text-[13.5px] font-semibold text-mujin-text hover:underline md:text-[14.5px]"
      href={href}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      ref={containerRef}
    >
      <span
        aria-hidden="true"
        className="pointer-events-none absolute invisible whitespace-nowrap"
        ref={textRef}
      >
        {title}
      </span>
      {!overflowing || !hovered ? (
        <span className="block truncate">{title}</span>
      ) : (
        <span className="player-title-marquee inline-flex max-w-none">
          <span>{title}</span>
          <span aria-hidden="true" className="pl-8">
            {title}
          </span>
        </span>
      )}
    </Link>
  );
}
