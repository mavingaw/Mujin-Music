"use client";

import { ExplicitBadge } from "@/features/library/song-title";
import { ArtistCreditLinks } from "@/features/library/artist-credit-links";
import { DeliveredFormat } from "@/features/streaming/delivered-format";
import { SourceBadge } from "@/features/streaming/source-badge";
import {
  isLiveRadioId,
  liveTitleText,
  parseStreamingId,
  type LiveNowPlaying,
} from "@parson/music-sdk";
import { LiveTimeline } from "@/features/live-radio/live-player";
import { LiveBadge, StationArtwork } from "@/features/live-radio/station-ui";
import type { TrackArtistCreditPart } from "@/features/library/track-artist-credit";

import Image from "next/image";
import { defaultCover } from "@/lib/images/default-cover";
import {
  ChevronDown,
  AudioLines,
  ChevronLeft,
  ListMusic,
  MicVocal,
  Minimize2,
  Radio,
} from "lucide-react";
import {
  memo,
  useRef,
  useState,
  type PointerEvent,
  type ReactNode,
  type RefObject,
} from "react";
import type { TimedLyric } from "./player-bar-hooks";
import type { Player } from "./player-api";
import {
  PlayerQueueList,
  QueueToolbar,
  type QueueEditing,
} from "./player-queue";
import { repeatModeLabel, type ZoneRepeatMode } from "@parson/music-sdk";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import {
  ActionButton,
  PlaybackControls,
  Timeline,
  VolumeControl,
} from "./player-controls";
import type { AudioPreset, AudioPresetId } from "./audio-presets";
import PlayerMoreMenu from "./player-more-menu";
import Link from "next/link";
import { rgbString, type ArtworkPalette } from "./artwork-palette";
import { useArtworkPalette } from "./use-artwork-palette";
import {
  DropdownMenuItem,
  DropdownMenuCheckboxItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "@/components/ui/dropdown-menu";

export interface NowPlayingDetails {
  albumId?: string;
  albumName?: string;
  year?: string;
  composers: string[];
  label?: string | null;
  genres?: string[];
  trackNumber?: number;
  trackCount?: number;
}

export interface UpNextItem {
  title: string;
  artist: string;
  cover: string;
}

/**
 * Roon's blurred-art backdrop, warmed with the colour taken from the cover:
 * a soft glow behind the artwork, deepening to near-black at the edges.
 */
function NowPlayingBackdrop({
  cover,
  dim,
  palette,
}: {
  cover: string;
  dim: boolean;
  palette: ArtworkPalette;
}) {
  return (
    <div
      aria-hidden="true"
      className="fixed inset-0 overflow-hidden bg-mujin-deep"
      style={{ backgroundColor: rgbString(palette.deep) }}
    >
      <div className="absolute -inset-[12%]">
        <Image
          alt=""
          className="scale-110 object-cover opacity-55 blur-[90px] saturate-[1.35]"
          fill
          sizes="40vw"
          src={cover}
        />
      </div>
      <div
        className="absolute inset-0 transition-opacity duration-500"
        style={{
          background: `radial-gradient(70% 60% at 28% 45%, ${rgbString(palette.glow, 0.28)}, transparent 70%), radial-gradient(120% 100% at 50% 110%, ${rgbString(palette.deep, 0.95)} 20%, transparent 70%)`,
        }}
      />
      <div
        className={`absolute inset-0 transition-colors duration-500 ${dim ? "bg-black/60" : "bg-black/35"}`}
      />
      <div className="absolute inset-0 bg-[radial-gradient(140%_100%_at_50%_40%,transparent_45%,rgba(5,5,6,0.85)_100%)]" />
    </div>
  );
}

/** The metadata stack under the title: album, year, composer, label. */
function NowPlayingFacts({
  details,
  onNavigate,
}: {
  details: NowPlayingDetails;
  onNavigate: () => void;
}) {
  const rows: { label: string; value: ReactNode }[] = [];
  if (details.albumName) {
    rows.push({
      label: "Album",
      value: details.albumId ? (
        <Link
          className="hover:text-mujin-text hover:underline"
          href={`/album?id=${encodeURIComponent(details.albumId)}`}
          onClick={onNavigate}
        >
          {details.albumName}
        </Link>
      ) : (
        details.albumName
      ),
    });
  }
  if (details.year) rows.push({ label: "Released", value: details.year });
  if (details.composers.length) {
    rows.push({
      label: details.composers.length > 1 ? "Composers" : "Composer",
      value: details.composers.join(", "),
    });
  }
  if (details.genres?.length) {
    rows.push({ label: "Genre", value: details.genres.slice(0, 3).join(", ") });
  }
  if (details.label) rows.push({ label: "Label", value: details.label });
  if (!rows.length) return null;
  return (
    <dl className="mt-6 grid max-w-md grid-cols-[auto_minmax(0,1fr)] gap-x-6 gap-y-1.5 border-t border-white/[0.08] pt-5 text-[13.5px] md:mt-8">
      {rows.map((row) => (
        <div className="contents" key={row.label}>
          <dt className="mj-eyebrow self-center text-[10px]!">{row.label}</dt>
          <dd className="truncate text-mujin-text-secondary">{row.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function FullscreenPlayer({
  activeLine,
  activeLineRef,
  artistCredits,
  audioPreset,
  audioPresets,
  cover,
  currentTime,
  duration,
  readPosition,
  isPlaying,
  looping,
  repeatMode,
  queueEditing,
  lyricsOpen,
  lyricsFallback,
  lyricsFailed,
  lyricsInstrumental,
  lyricsLoading,
  lyricsScrollRef,
  muted,
  onClose,
  onNext,
  onOpenLyrics,
  onOpenQueue,
  onBackToPlayer,
  onPrevious,
  onRetryLyrics,
  onReorderQueue,
  onSelectQueueItem,
  onSeek,
  onSelectPreset,
  onToggleLoop,
  onToggleMute,
  onTogglePlayback,
  onToggleSound,
  onVolumeChange,
  playbackRate,
  queue,
  queueOpen,
  signalBadge,
  slowedReverb,
  songId,
  title,
  explicit,
  feedback,
  radioLabel,
  timedLyrics,
  volume,
  zonePicker,
  livePlaying,
  details,
  upNext,
}: {
  activeLine: number;
  activeLineRef: RefObject<HTMLElement | null>;
  artistCredits: TrackArtistCreditPart[];
  audioPreset: AudioPresetId;
  audioPresets: readonly AudioPreset[];
  cover: string;
  currentTime: number;
  duration: number;
  readPosition?: () => number;
  isPlaying: boolean;
  looping: boolean;
  repeatMode?: ZoneRepeatMode;
  queueEditing?: QueueEditing;
  lyricsOpen: boolean;
  lyricsFallback?: string | null;
  lyricsFailed: boolean;
  lyricsInstrumental: boolean;
  lyricsLoading: boolean;
  lyricsScrollRef: RefObject<HTMLDivElement | null>;
  muted: boolean;
  onClose: () => void;
  onNext: () => void;
  onOpenLyrics: () => void;
  onOpenQueue: () => void;
  onBackToPlayer: () => void;
  onPrevious: () => void;
  onRetryLyrics: () => void;
  onReorderQueue: (from: number, to: number) => void;
  onSelectQueueItem: (index: number) => void;
  onSeek: (value: number) => void;
  onSelectPreset: (value: AudioPresetId) => void;
  onToggleLoop: () => void;
  onToggleMute: () => void;
  onTogglePlayback: () => void;
  onToggleSound: () => void;
  onVolumeChange: (value: number) => void;
  playbackRate: number;
  queue: Player["queue"];
  queueOpen: boolean;
  slowedReverb: boolean;
  songId: string;
  title: string;
  explicit: boolean;
  /** Thumbs up / down for the current track. */
  feedback?: ReactNode;
  /** "Radio · Playing X radio" when the current track is a Radio pick. */
  radioLabel?: string | null;
  /** Signal path pill shown under the artist. */
  signalBadge?: ReactNode;
  timedLyrics: TimedLyric[];
  volume: number;
  /** Zone picker; on phones this is where zones are reached. */
  zonePicker?: ReactNode;
  /** What a live station is playing, when one plays. */
  livePlaying?: LiveNowPlaying | null;
  /** Album, year, composer and label for the metadata stack. */
  details?: NowPlayingDetails;
  /** The next track in the queue. */
  upNext?: UpNextItem | null;
}) {
  const live = isLiveRadioId(songId);
  const palette = useArtworkPalette(cover);
  const dragStart = useRef<number | null>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const [dragOffset, setDragOffset] = useState(0);
  const mobileView = lyricsOpen
    ? "Lyrics"
    : queueOpen
      ? "Queue"
      : "Now playing";
  const showingMobileSubView = lyricsOpen || queueOpen;
  const streamingSource = parseStreamingId(songId);

  const beginDrag = (event: PointerEvent<HTMLDivElement>) => {
    dragStart.current = event.clientY;
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const drag = (event: PointerEvent<HTMLDivElement>) => {
    if (dragStart.current === null) {
      return;
    }
    setDragOffset(Math.max(0, event.clientY - dragStart.current));
  };
  const endDrag = () => {
    dragStart.current = null;
    if (dragOffset > 88) {
      onClose();
    } else {
      setDragOffset(0);
    }
  };

  return (
    <DialogPrimitive.Root
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Content
          asChild
          aria-describedby={undefined}
          aria-labelledby={undefined}
          onOpenAutoFocus={() => {
            returnFocus.current =
              document.activeElement instanceof HTMLElement
                ? document.activeElement
                : null;
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const trigger = returnFocus.current?.isConnected
              ? returnFocus.current
              : document.querySelector<HTMLElement>(
                  'footer button[aria-label="More controls"]',
                );
            trigger?.focus();
          }}
          onEscapeKeyDown={(event) => {
            if (lyricsOpen || queueOpen) {
              event.preventDefault();
              onBackToPlayer();
            }
          }}
        >
          <section
            aria-label="Fullscreen player"
            className="fixed inset-0 z-70 flex min-h-dvh flex-col overflow-hidden bg-black text-white motion-safe:animate-in motion-safe:slide-in-from-bottom motion-safe:duration-300 md:overflow-y-auto md:animate-none"
            role="dialog"
            style={{
              transform: dragOffset ? `translateY(${dragOffset}px)` : undefined,
              transition:
                dragStart.current === null
                  ? "transform 220ms ease-out"
                  : "none",
            }}
          >
            <DialogPrimitive.Title className="sr-only">
              Now playing
            </DialogPrimitive.Title>
            <NowPlayingBackdrop
              cover={cover}
              dim={lyricsOpen || queueOpen}
              palette={palette}
            />
            <div
              className="relative z-20 flex touch-none select-none items-center justify-between px-4 pb-2 pt-[max(10px,env(safe-area-inset-top))] md:hidden"
              onPointerCancel={endDrag}
              onPointerDown={beginDrag}
              onPointerMove={drag}
              onPointerUp={endDrag}
            >
              <button
                aria-label={
                  showingMobileSubView
                    ? "Return to now playing"
                    : "Close now playing"
                }
                className="grid h-11 w-11 place-items-center rounded-full active:bg-white/10"
                onClick={showingMobileSubView ? onBackToPlayer : onClose}
                type="button"
              >
                {showingMobileSubView ? (
                  <ChevronLeft className="h-7 w-7" />
                ) : (
                  <ChevronDown className="h-7 w-7" />
                )}
              </button>
              <div className="text-center">
                <p className="text-sm font-medium text-zinc-500">
                  {mobileView}
                </p>
              </div>
              <div aria-hidden="true" className="h-11 w-11" />
            </div>
            <button
              aria-label="Exit fullscreen player"
              className="fixed right-6 top-6 z-30 hidden h-10 w-10 items-center justify-center rounded-full border border-white/10 bg-black/30 text-mujin-text-secondary backdrop-blur-md transition-colors hover:bg-white/10 hover:text-mujin-text md:flex sm:right-8"
              onClick={onClose}
              title="Exit fullscreen"
              type="button"
            >
              <Minimize2 className="h-5 w-5" />
            </button>
            <div className="relative z-10 flex min-h-0 flex-1 flex-col overflow-hidden">
              {lyricsOpen ? (
                <div className="mx-auto min-h-0 w-full max-w-190 flex-1 overflow-hidden px-2 md:px-6">
                  <LyricsContent
                    activeLine={activeLine}
                    activeLineRef={activeLineRef}
                    fallback={lyricsFallback}
                    failed={lyricsFailed}
                    instrumental={lyricsInstrumental}
                    loading={lyricsLoading}
                    onRetry={onRetryLyrics}
                    onSeek={onSeek}
                    scrollRef={lyricsScrollRef}
                    timed={timedLyrics}
                  />
                </div>
              ) : queueOpen ? (
                <div className="mx-auto min-h-0 w-full max-w-160 flex-1 overflow-y-auto px-4 pt-4 md:pt-16">
                  <div className="mb-4 flex items-center justify-between gap-3">
                    <h2 className="mj-title text-[28px] text-mujin-text">
                      Queue
                    </h2>
                    {queueEditing && (
                      <QueueToolbar
                        editing={queueEditing}
                        length={queue.length}
                      />
                    )}
                  </div>
                  {queue.length ? (
                    <PlayerQueueList
                      currentSongId={songId}
                      editing={queueEditing}
                      onReorder={onReorderQueue}
                      onSelect={onSelectQueueItem}
                      queue={queue}
                    />
                  ) : (
                    <p className="py-8 text-sm text-zinc-500">Nothing queued</p>
                  )}
                </div>
              ) : (
                <div className="mx-auto flex min-h-0 w-full max-w-[1320px] flex-1 flex-col items-center justify-start gap-6 overflow-y-auto px-6 py-5 md:grid md:grid-cols-2 md:content-center md:items-center md:gap-[clamp(32px,5vw,96px)] md:px-12 md:py-[clamp(16px,4dvh,48px)]">
                  <div
                    className="relative mx-auto aspect-square w-full max-w-[min(78vw,340px,max(120px,calc(100dvh-360px)))] shrink-0 overflow-hidden rounded-[8px] bg-mujin-surface-1 md:mx-0 md:max-w-[min(560px,calc(100dvh-230px))] md:justify-self-end"
                    style={{
                      boxShadow: `0 0 0 1px rgb(242 238 230 / 0.08), 0 50px 120px -40px ${rgbString(palette.glow, 0.55)}, 0 30px 60px -30px rgb(0 0 0 / 0.9)`,
                    }}
                  >
                    {live ? (
                      <StationArtwork
                        className="absolute inset-0 h-full w-full"
                        rounded="rounded-[8px]"
                        sizes="560px"
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
                        priority
                        sizes="(min-width: 768px) 560px, 80vw"
                        src={cover}
                      />
                    )}
                  </div>
                  <div className="w-full min-w-0 max-w-[560px] md:justify-self-start">
                    {live ? (
                      <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-rose-300/90">
                        <LiveBadge /> Live radio
                      </p>
                    ) : radioLabel ? (
                      <p className="mb-3 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.14em] text-mujin-copper">
                        <Radio aria-hidden="true" className="h-3.5 w-3.5" />
                        <span className="truncate">{radioLabel}</span>
                      </p>
                    ) : (
                      <p className="mj-eyebrow mb-3 hidden md:block">
                        Now playing
                        {details?.trackNumber && details.trackCount
                          ? ` · Track ${details.trackNumber} of ${details.trackCount}`
                          : ""}
                      </p>
                    )}
                    <h1
                      className="mj-display wrap-break-word text-[32px] text-mujin-text md:text-[clamp(38px,min(5vw,7dvh),72px)]"
                      title={title}
                    >
                      {title}
                    </h1>
                    <p className="mt-2 text-[16px] leading-7 text-mujin-text-secondary md:mt-4 md:text-[22px] md:leading-8">
                      {explicit && <ExplicitBadge />}
                      <ArtistCreditLinks
                        credits={artistCredits}
                        onNavigate={onClose}
                      />
                    </p>
                    {details && !live && (
                      <NowPlayingFacts details={details} onNavigate={onClose} />
                    )}
                    {(signalBadge || feedback) && (
                      <div className="mt-6 flex min-w-0 flex-wrap items-center gap-3">
                        {signalBadge}
                        {feedback}
                      </div>
                    )}
                    {live && (livePlaying?.history.length ?? 0) > 0 && (
                      <div className="mt-6">
                        <p className="mj-eyebrow">Earlier on {title}</p>
                        <ol className="mt-2 space-y-1 text-sm text-mujin-text-secondary">
                          {livePlaying!.history.slice(0, 4).map((item) => (
                            <li
                              className="truncate"
                              key={`${item.at}-${item.raw}`}
                            >
                              {liveTitleText(item)}
                            </li>
                          ))}
                        </ol>
                      </div>
                    )}
                    {streamingSource && (
                      <div className="mt-4 flex items-center gap-2.5">
                        <SourceBadge provider={streamingSource.provider} />
                        <DeliveredFormat songId={songId} />
                      </div>
                    )}
                    {upNext && !live && (
                      <button
                        className="group mt-8 hidden w-full max-w-md items-center gap-3 rounded-lg border border-white/[0.07] bg-black/25 p-2.5 pr-4 text-left backdrop-blur-md transition-colors hover:border-white/15 hover:bg-black/35 md:flex"
                        onClick={onOpenQueue}
                        type="button"
                      >
                        <span className="relative h-11 w-11 shrink-0 overflow-hidden rounded-[4px]">
                          <Image
                            alt=""
                            className="object-cover"
                            fill
                            sizes="44px"
                            src={upNext.cover}
                          />
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="mj-eyebrow block text-[10px]!">
                            Up next
                          </span>
                          <span className="block truncate text-[13.5px] font-medium text-mujin-text">
                            {upNext.title}
                          </span>
                          <span className="block truncate text-[12px] text-mujin-text-tertiary">
                            {upNext.artist}
                          </span>
                        </span>
                        <ListMusic className="h-4 w-4 shrink-0 text-mujin-text-tertiary transition-colors group-hover:text-mujin-text" />
                      </button>
                    )}
                  </div>
                </div>
              )}
            </div>
            <div className="relative z-20 mx-auto w-full max-w-[920px] shrink-0 px-6 pb-[max(20px,env(safe-area-inset-bottom))] pt-3 md:px-10 md:pb-8">
              {showingMobileSubView && (
                <p className="mb-3 truncate text-center text-xs text-zinc-500">
                  {title}
                </p>
              )}
              {live ? (
                <LiveTimeline
                  className="mb-3 flex w-full"
                  currentTime={currentTime}
                  isPlaying={isPlaying}
                  playing={livePlaying}
                />
              ) : (
                <Timeline
                  className="mb-3 flex w-full"
                  currentTime={currentTime}
                  duration={duration}
                  isPlaying={isPlaying}
                  readPosition={readPosition}
                  onChange={onSeek}
                  playbackRate={playbackRate}
                />
              )}
              <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-4">
                <div className="flex items-center gap-1">
                  <button
                    aria-label="Lyrics"
                    disabled={live}
                    aria-pressed={lyricsOpen}
                    className={`flex h-9 w-fit items-center gap-2 rounded-full px-3 text-xs font-medium transition-colors ${lyricsOpen ? "bg-mujin-copper/[0.16] text-mujin-copper" : "text-mujin-text-secondary hover:bg-white/[0.06] hover:text-mujin-text"}`}
                    onClick={onOpenLyrics}
                    type="button"
                  >
                    <MicVocal className="h-4 w-4" />
                    <span className="hidden sm:inline">Lyrics</span>
                  </button>
                  <button
                    aria-label={queueOpen ? "Back to now playing" : "Queue"}
                    aria-pressed={queueOpen}
                    className={`hidden h-9 w-fit items-center gap-2 rounded-full px-3 text-xs font-medium transition-colors sm:flex ${queueOpen ? "bg-mujin-copper/[0.16] text-mujin-copper" : "text-mujin-text-secondary hover:bg-white/[0.06] hover:text-mujin-text"}`}
                    onClick={queueOpen ? onBackToPlayer : onOpenQueue}
                    type="button"
                  >
                    <ListMusic className="h-4 w-4" />
                    <span className="hidden lg:inline">Queue</span>
                  </button>
                </div>
                <PlaybackControls
                  prominent
                  isPlaying={isPlaying}
                  onNext={onNext}
                  onPrevious={onPrevious}
                  onToggle={onTogglePlayback}
                />
                <div className="flex items-center justify-self-end gap-1 sm:gap-1.5">
                  <ActionButton
                    active={slowedReverb}
                    label="Sound"
                    onClick={onToggleSound}
                  >
                    <AudioLines className="h-4 w-4" />
                  </ActionButton>
                  {zonePicker}
                  <div className="hidden sm:block">
                    <VolumeControl
                      muted={muted}
                      volume={volume}
                      onChange={onVolumeChange}
                      onToggleMute={onToggleMute}
                    />
                  </div>
                  <PlayerMoreMenu label="More player controls">
                    <DropdownMenuItem
                      onSelect={queueOpen ? onBackToPlayer : onOpenQueue}
                    >
                      <ListMusic />
                      {queueOpen ? "Now playing" : "Queue"}
                    </DropdownMenuItem>
                    <DropdownMenuCheckboxItem
                      checked={
                        (repeatMode ?? (looping ? "one" : "off")) !== "off"
                      }
                      onCheckedChange={onToggleLoop}
                    >
                      {repeatModeLabel(repeatMode ?? (looping ? "one" : "off"))}
                    </DropdownMenuCheckboxItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuLabel>Sound</DropdownMenuLabel>
                    <DropdownMenuRadioGroup
                      value={audioPreset}
                      onValueChange={(value) =>
                        onSelectPreset(value as AudioPresetId)
                      }
                    >
                      {audioPresets.map((preset) => (
                        <DropdownMenuRadioItem
                          key={preset.id}
                          value={preset.id}
                        >
                          {preset.label}
                        </DropdownMenuRadioItem>
                      ))}
                    </DropdownMenuRadioGroup>
                  </PlayerMoreMenu>
                </div>
              </div>
            </div>
          </section>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

export const LyricsPanel = memo(function LyricsPanel({
  activeLine,
  activeLineRef,
  cover,
  onSeek,
  fallback,
  failed,
  instrumental,
  loading,
  onRetry,
  scrollRef,
  timed,
}: {
  activeLine: number;
  activeLineRef: RefObject<HTMLElement | null>;
  onSeek: (value: number) => void;
  cover: string;
  fallback?: string | null;
  failed: boolean;
  instrumental: boolean;
  loading: boolean;
  onRetry: () => void;
  scrollRef: RefObject<HTMLDivElement | null>;
  timed: TimedLyric[];
}) {
  return (
    <section
      aria-label="Lyrics"
      className="app-content-shell z-40 overflow-hidden bg-black"
    >
      <LyricsBackground cover={cover} />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 z-1 h-24 bg-linear-to-t from-black/45 to-transparent mask-[linear-gradient(to_top,black,transparent)]" />
      <div className="relative z-2 mx-auto h-full max-w-180">
        <LyricsContent
          activeLine={activeLine}
          activeLineRef={activeLineRef}
          fallback={fallback}
          failed={failed}
          instrumental={instrumental}
          loading={loading}
          onRetry={onRetry}
          onSeek={onSeek}
          scrollRef={scrollRef}
          timed={timed}
        />
      </div>
    </section>
  );
});

function LyricsBackground({ cover }: { cover: string }) {
  return (
    <>
      <div className="absolute -inset-12">
        <Image
          alt=""
          aria-hidden="true"
          className="object-cover opacity-50 blur-3xl"
          fill
          sizes="100vw"
          src={cover}
        />
      </div>
      <div className="absolute inset-0 bg-black/55" />
    </>
  );
}

const LyricsContent = memo(function LyricsContent({
  activeLine,
  activeLineRef,
  fallback,
  failed,
  instrumental,
  loading,
  onRetry,
  onSeek,
  scrollRef,
  timed,
}: {
  activeLine: number;
  activeLineRef: RefObject<HTMLElement | null>;
  fallback?: string | null;
  failed: boolean;
  instrumental: boolean;
  loading: boolean;
  onRetry: () => void;
  onSeek: (value: number) => void;
  scrollRef: RefObject<HTMLDivElement | null>;
  timed: TimedLyric[];
}) {
  return (
    <div
      ref={scrollRef}
      className="h-full overflow-y-auto px-7 py-8 text-center scrollbar-none sm:px-10 [&::-webkit-scrollbar]:hidden"
    >
      {loading ? (
        <div className="grid h-full place-items-center">
          <div className="h-2 w-2 animate-pulse rounded-full bg-white/70" />
        </div>
      ) : failed ? (
        <div className="grid h-full place-items-center">
          <div className="max-w-sm">
            <p className="text-2xl font-semibold text-white">
              Lyrics are temporarily unavailable
            </p>
            <button
              className="mj-primary mt-5 rounded-full px-5 py-2.5 text-sm font-semibold"
              onClick={onRetry}
              type="button"
            >
              Try again
            </button>
          </div>
        </div>
      ) : timed.length ? (
        <div className="space-y-5 pb-[50vh] pt-1">
          {timed.map((line, index) =>
            line.text.trim() ? (
              <button
                key={`${line.time}-${index}`}
                ref={(node) => {
                  if (index === activeLine) {
                    activeLineRef.current = node;
                  }
                }}
                className={`mj-title block w-full cursor-pointer text-center text-[26px] leading-snug transition-[color,transform] duration-300 hover:text-mujin-text sm:text-[36px] ${index === activeLine ? "text-mujin-text motion-safe:scale-[1.03]" : index < activeLine ? "text-mujin-text/25" : "text-mujin-text/45"}`}
                onClick={() => onSeek(line.time)}
                type="button"
              >
                {line.text}
              </button>
            ) : (
              <div
                aria-hidden="true"
                className="h-5"
                key={`${line.time}-${index}`}
                ref={(node) => {
                  if (index === activeLine) {
                    activeLineRef.current = node;
                  }
                }}
              />
            ),
          )}
        </div>
      ) : fallback ? (
        <p className="mj-title whitespace-pre-line py-8 text-[26px] leading-[1.6] text-mujin-text sm:text-[34px]">
          {fallback}
        </p>
      ) : (
        <div className="grid h-full place-items-center">
          <p className="mj-display text-[34px] leading-[1.4] text-mujin-text/80 sm:text-[48px]">
            {instrumental
              ? "Instrumental"
              : "No lyrics are available for this track."}
          </p>
        </div>
      )}
    </div>
  );
});
