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
  type LyricsFailure,
  type LyricsSourceKind,
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
  ChevronRight,
  ListMusic,
  MicVocal,
  Minimize2,
  Radio,
  Repeat,
  Repeat1,
  Shuffle,
  Tv,
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
import { LyricsBackdrop, LyricsSourceChip, LyricsView } from "./lyrics-view";
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
import { nowPlayingScene, type NowPlayingScene } from "./now-playing-scene";
import { useArtworkPalette } from "./use-artwork-palette";
import { useOptionalAppearance } from "@/features/appearance/theme-provider";
import { cn } from "@/lib/cn";
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
 * a soft glow behind the artwork, deepening to near-black at the edges. The
 * theme decides how much of the cover shows (see `nowPlayingScene`): true
 * black for OLED Black, the plain canvas for High Contrast, and a brighter
 * blur under glass panels for Frosted Glass.
 */
function NowPlayingBackdrop({
  cover,
  immersive,
  scene,
}: {
  cover: string;
  /** Immersive layout: the cover itself fills the screen, sharp. */
  immersive: boolean;
  scene: NowPlayingScene;
}) {
  return (
    <div
      aria-hidden="true"
      className="fixed inset-0 overflow-hidden bg-mujin-deep"
      data-np-backdrop={
        scene.showArt ? (immersive ? "immersive" : "blur") : scene.backdrop
      }
      style={{ backgroundColor: scene.base }}
    >
      {scene.showArt &&
        (immersive ? (
          <div className="absolute inset-0">
            <Image
              alt=""
              className="object-cover object-center"
              fill
              priority
              sizes="100vw"
              src={cover}
            />
          </div>
        ) : (
          <div className="absolute -inset-[12%]">
            <Image
              alt=""
              className={cn(
                "scale-110 object-cover blur-[90px] saturate-[1.35]",
                scene.panels === "glass"
                  ? "opacity-80"
                  : scene.base
                    ? "opacity-55"
                    : "opacity-40",
              )}
              fill
              sizes="40vw"
              src={cover}
            />
          </div>
        ))}
      {scene.wash && (
        <div
          className="absolute inset-0 transition-opacity duration-500"
          style={{ background: scene.wash }}
        />
      )}
      {scene.dim && (
        <div
          className={`absolute inset-0 transition-colors duration-500 ${scene.dim}`}
        />
      )}
      {scene.vignette && (
        <div
          className="absolute inset-0"
          style={{ background: scene.vignette }}
        />
      )}
    </div>
  );
}

/** Phones: one line of album and year instead of the full stack. */
function PhoneNowPlayingMeta({ details }: { details: NowPlayingDetails }) {
  const parts = [details.albumName, details.year].filter(Boolean);
  if (!parts.length) return null;
  return (
    <p className="mt-1 truncate text-[0.8125rem] text-mujin-text-tertiary md:hidden">
      {parts.join(" · ")}
    </p>
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
    <dl className="mj-np-facts mt-6 hidden max-w-md grid-cols-[auto_minmax(0,1fr)] gap-x-6 gap-y-1.5 border-t border-white/[0.08] pt-5 text-[0.8438rem] md:mt-8 md:grid">
      {rows.map((row) => (
        <div className="contents" key={row.label}>
          <dt className="mj-eyebrow self-center text-[0.625rem]!">
            {row.label}
          </dt>
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
  lyricsFailure,
  lyricsInstrumental,
  lyricsLoading,
  lyricsScrollRef,
  lyricsSource,
  lyricsBackdrop,
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
  upNextItems,
  artistBio,
  favorite,
  shuffle,
  onToggleShuffle,
  onOpenDisplay,
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
  lyricsFailure: LyricsFailure | null;
  lyricsInstrumental: boolean;
  lyricsLoading: boolean;
  lyricsScrollRef: RefObject<HTMLDivElement | null>;
  lyricsSource: LyricsSourceKind | null;
  /** The artist photo behind the lyrics; the blurred cover without one. */
  lyricsBackdrop?: string | null;
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
  /** A short "About the artist" card from their biography. */
  artistBio?: ReactNode;
  /** The next few tracks, for the phone's "Up next" card. */
  upNextItems?: UpNextItem[];
  /** The song heart. */
  favorite?: ReactNode;
  shuffle?: boolean;
  /** Turns shuffle on or off; `null` where the zone cannot shuffle. */
  onToggleShuffle?: (() => void) | null;
  /** Opens this zone's full-screen display (TV mode) in a new window. */
  onOpenDisplay?: () => void;
}) {
  const live = isLiveRadioId(songId);
  const palette = useArtworkPalette(cover);
  const appearance = useOptionalAppearance();
  const scene = nowPlayingScene({
    backdrop: appearance?.theme.backdrop ?? "artwork",
    dim: queueOpen,
    layout: appearance?.appearance.now_playing_layout ?? "art-left",
    light: appearance?.theme.mode === "light",
    palette,
  });
  const layout = scene.layout;
  const panels = scene.panels;
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
            data-np-scene={scene.backdrop}
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
            {lyricsOpen ? (
              <div className="fixed inset-0 bg-mujin-deep">
                <LyricsBackdrop cover={cover} photo={lyricsBackdrop} />
              </div>
            ) : (
              <NowPlayingBackdrop
                cover={cover}
                immersive={layout === "immersive" && !queueOpen}
                scene={scene}
              />
            )}
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
                <div className="mx-auto grid min-h-0 w-full max-w-[1320px] flex-1 md:grid-cols-[minmax(240px,360px)_minmax(0,1fr)] md:gap-[clamp(32px,5vw,88px)] md:px-12">
                  <aside className="hidden min-w-0 flex-col justify-center md:flex">
                    <div
                      className="relative aspect-square w-full max-w-[300px] overflow-hidden rounded-[calc(8px*var(--mj-radius-scale,1))] bg-mujin-surface-1"
                      style={{
                        boxShadow: scene.coverShadow,
                      }}
                    >
                      <Image
                        alt=""
                        className="object-cover"
                        fill
                        sizes="300px"
                        src={cover}
                      />
                    </div>
                    <p className="mj-eyebrow mt-7">Lyrics</p>
                    <h2
                      className="mj-display mt-2 line-clamp-2 text-[1.875rem] leading-tight text-mujin-text lg:text-[2.25rem]"
                      title={title}
                    >
                      {title}
                    </h2>
                    <p className="mt-2 truncate text-[1rem] text-mujin-text-secondary">
                      <ArtistCreditLinks
                        credits={artistCredits}
                        onNavigate={onClose}
                      />
                    </p>
                    {!lyricsLoading &&
                      (timedLyrics.length > 0 || lyricsFallback) && (
                        <div className="mt-5">
                          <LyricsSourceChip
                            source={lyricsSource}
                            timed={timedLyrics.length > 0}
                          />
                        </div>
                      )}
                  </aside>
                  <div className="flex min-h-0 flex-col overflow-hidden">
                    {!lyricsLoading &&
                      (timedLyrics.length > 0 || lyricsFallback) && (
                        <div className="flex justify-center px-6 pb-1 md:hidden">
                          <LyricsSourceChip
                            source={lyricsSource}
                            timed={timedLyrics.length > 0}
                          />
                        </div>
                      )}
                    <div className="min-h-0 flex-1">
                      <LyricsView
                        activeLine={activeLine}
                        activeLineRef={activeLineRef}
                        fallback={lyricsFallback}
                        failure={lyricsFailure}
                        instrumental={lyricsInstrumental}
                        loading={lyricsLoading}
                        onRetry={onRetryLyrics}
                        onSeek={onSeek}
                        scrollRef={lyricsScrollRef}
                        source={lyricsSource}
                        timed={timedLyrics}
                      />
                    </div>
                  </div>
                </div>
              ) : queueOpen ? (
                <div className="mx-auto min-h-0 w-full max-w-160 flex-1 overflow-y-auto px-4 pt-4 md:pt-16">
                  <div className="mb-4 flex items-center justify-between gap-3">
                    <h2 className="mj-title text-[1.75rem] text-mujin-text">
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
                <div
                  className={cn(
                    "mx-auto flex min-h-0 w-full max-w-[1320px] flex-1 flex-col items-center justify-start gap-5 overflow-y-auto px-6 pb-6 pt-3 max-md:[mask-image:linear-gradient(to_bottom,black_calc(100%-20px),transparent)] md:gap-6 md:px-12 md:py-[clamp(16px,4dvh,48px)]",
                    layout === "art-left" &&
                      "md:grid md:grid-cols-2 md:content-center md:items-center md:gap-[clamp(32px,5vw,96px)]",
                    layout === "art-centred" &&
                      "md:justify-center md:gap-[clamp(20px,3dvh,36px)] md:text-center",
                    layout === "immersive" &&
                      "justify-end md:items-start md:justify-end md:pb-[clamp(28px,6dvh,72px)]",
                  )}
                  data-np-layout={layout}
                >
                  <div
                    className={cn(
                      "relative mx-auto aspect-square w-full max-w-[min(78vw,340px,max(120px,calc(100dvh-360px)))] shrink-0 overflow-hidden rounded-[calc(8px*var(--mj-radius-scale,1))] bg-mujin-surface-1",
                      layout === "art-left" &&
                        "md:mx-0 md:max-w-[min(560px,calc(100dvh-230px))] md:justify-self-end",
                      layout === "art-centred" &&
                        "md:max-w-[min(440px,calc(100dvh-380px))]",
                      layout === "immersive" && "hidden",
                    )}
                    style={{
                      boxShadow: scene.coverShadow,
                    }}
                  >
                    {live ? (
                      <StationArtwork
                        className="absolute inset-0 h-full w-full"
                        rounded="rounded-[calc(8px*var(--mj-radius-scale,1))]"
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
                  <div
                    className={cn(
                      "w-full min-w-0 max-w-[560px]",
                      panels !== "plain" &&
                        "mj-np-info md:rounded-[calc(14px*var(--mj-radius-scale,1))] md:p-8",
                      layout === "art-left" && "md:justify-self-start",
                      layout === "art-centred" &&
                        "md:max-w-[680px] [&_.mj-facts]:md:justify-center",
                      layout === "immersive" && "md:max-w-[760px]",
                    )}
                  >
                    {live ? (
                      <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-rose-300/90">
                        <LiveBadge /> Live radio
                      </p>
                    ) : radioLabel ? (
                      <p className="mb-3 flex items-center gap-1.5 text-[0.6875rem] font-semibold uppercase tracking-[0.14em] text-mujin-copper">
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
                    <div className="flex items-start gap-3">
                      <div className="min-w-0 flex-1">
                        <h1
                          className="mj-display wrap-break-word text-[2rem] text-mujin-text md:text-[clamp(38px,min(5vw,7dvh),72px)]"
                          title={title}
                        >
                          {title}
                        </h1>
                        <p className="mt-2 text-[1rem] leading-7 text-mujin-text-secondary md:mt-4 md:text-[1.375rem] md:leading-8">
                          {explicit && <ExplicitBadge />}
                          <ArtistCreditLinks
                            credits={artistCredits}
                            onNavigate={onClose}
                          />
                        </p>
                      </div>
                      {favorite && !live && (
                        <div className="-mr-2 mt-1 shrink-0 md:hidden [&_button]:h-11 [&_button]:w-11 [&_svg]:h-5.5 [&_svg]:w-5.5">
                          {favorite}
                        </div>
                      )}
                    </div>
                    {details && !live && (
                      <>
                        <PhoneNowPlayingMeta details={details} />
                        <NowPlayingFacts
                          details={details}
                          onNavigate={onClose}
                        />
                      </>
                    )}
                    {(signalBadge || feedback || favorite) && (
                      <div className="mt-4 flex min-w-0 flex-wrap items-center gap-3 md:mt-6">
                        {signalBadge}
                        {feedback}
                        {favorite && !live && (
                          <span className="hidden md:contents">{favorite}</span>
                        )}
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
                    {!live && upNextItems && upNextItems.length > 0 && (
                      <section
                        aria-label="Up next"
                        className="mj-np-card mt-5 rounded-lg border border-white/[0.07] bg-black/25 backdrop-blur-md md:hidden"
                      >
                        <button
                          className="flex w-full items-center justify-between px-3.5 pb-1 pt-3 text-left"
                          onClick={onOpenQueue}
                          type="button"
                        >
                          <span className="mj-eyebrow text-[0.625rem]!">
                            Up next
                          </span>
                          <span className="flex items-center gap-0.5 text-[0.75rem] text-mujin-text-secondary">
                            Queue
                            <ChevronRight
                              aria-hidden="true"
                              className="h-3.5 w-3.5"
                            />
                          </span>
                        </button>
                        <ol className="px-1.5 pb-1.5">
                          {upNextItems.slice(0, 3).map((item, index) => (
                            <li
                              className={cn(
                                // One row always fits above the transport;
                                // taller phones show the next two as well.
                                index === 1 &&
                                  "[@media(max-height:979px)]:hidden",
                                index === 2 &&
                                  "[@media(max-height:1059px)]:hidden",
                              )}
                              key={`${index}-${item.title}`}
                            >
                              <button
                                className="flex w-full items-center gap-3 rounded-md p-2 text-left active:bg-white/[0.06]"
                                onClick={onOpenQueue}
                                type="button"
                              >
                                <span className="relative h-10 w-10 shrink-0 overflow-hidden rounded-[4px] bg-mujin-surface-2">
                                  <Image
                                    alt=""
                                    className="object-cover"
                                    fill
                                    sizes="40px"
                                    src={item.cover}
                                  />
                                </span>
                                <span className="min-w-0 flex-1">
                                  <span className="block truncate text-[0.875rem] text-mujin-text">
                                    {item.title}
                                  </span>
                                  <span className="block truncate text-[0.75rem] text-mujin-text-tertiary">
                                    {item.artist}
                                  </span>
                                </span>
                              </button>
                            </li>
                          ))}
                        </ol>
                      </section>
                    )}
                    {!live && artistBio && (
                      <div className="hidden md:block">{artistBio}</div>
                    )}
                    {upNext && !live && (
                      <button
                        className="mj-np-card group mt-6 hidden w-full max-w-md items-center gap-3 rounded-lg border border-white/[0.07] bg-black/25 p-2.5 pr-4 text-left backdrop-blur-md transition-colors hover:border-white/15 hover:bg-black/35 md:flex"
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
                          <span className="mj-eyebrow block text-[0.625rem]!">
                            Up next
                          </span>
                          <span className="block truncate text-[0.8438rem] font-medium text-mujin-text">
                            {upNext.title}
                          </span>
                          <span className="block truncate text-[0.75rem] text-mujin-text-tertiary">
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
            <div
              className={cn(
                "relative z-20 mx-auto w-full max-w-[920px] shrink-0 px-6 pb-[max(20px,env(safe-area-inset-bottom))] pt-3 md:px-10 md:pb-8",
                panels !== "plain" &&
                  "mj-np-transport max-md:rounded-t-[calc(18px*var(--mj-radius-scale,1))] md:mb-6 md:w-[calc(100%-48px)] md:rounded-[calc(18px*var(--mj-radius-scale,1))] md:pb-5 md:pt-4",
              )}
            >
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
              <PhoneTransport
                isPlaying={isPlaying}
                live={live}
                lyricsOpen={lyricsOpen}
                onNext={onNext}
                onOpenLyrics={onOpenLyrics}
                onOpenQueue={queueOpen ? onBackToPlayer : onOpenQueue}
                onPrevious={onPrevious}
                onToggleLoop={onToggleLoop}
                onTogglePlayback={onTogglePlayback}
                onToggleShuffle={onToggleShuffle}
                queueOpen={queueOpen}
                repeatMode={repeatMode ?? (looping ? "one" : "off")}
                shuffle={Boolean(shuffle)}
                more={
                  <PlayerMoreMenu label="More player controls">
                    <DropdownMenuItem onSelect={onToggleSound}>
                      <AudioLines />
                      {slowedReverb ? "Sound: effect on" : "Sound"}
                    </DropdownMenuItem>
                    {onOpenDisplay && (
                      <DropdownMenuItem onSelect={onOpenDisplay}>
                        <Tv />
                        Open as display
                      </DropdownMenuItem>
                    )}
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
                }
                zonePicker={zonePicker}
              />
              <div className="hidden grid-cols-[1fr_auto_1fr] items-center gap-4 md:grid">
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
                    {onToggleShuffle && (
                      <DropdownMenuCheckboxItem
                        checked={Boolean(shuffle)}
                        onCheckedChange={onToggleShuffle}
                      >
                        Shuffle
                      </DropdownMenuCheckboxItem>
                    )}
                    {onOpenDisplay && (
                      <DropdownMenuItem onSelect={onOpenDisplay}>
                        <Tv />
                        Open as display
                      </DropdownMenuItem>
                    )}
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

/**
 * The phone's controls under the seek bar: shuffle, previous, play, next
 * and repeat in one row, then the zone, lyrics, queue and more, so nothing
 * needs a trip out of Now Playing.
 */
function PhoneTransport({
  isPlaying,
  live,
  lyricsOpen,
  more,
  onNext,
  onOpenLyrics,
  onOpenQueue,
  onPrevious,
  onToggleLoop,
  onTogglePlayback,
  onToggleShuffle,
  queueOpen,
  repeatMode,
  shuffle,
  zonePicker,
}: {
  isPlaying: boolean;
  live: boolean;
  lyricsOpen: boolean;
  more: ReactNode;
  onNext: () => void;
  onOpenLyrics: () => void;
  onOpenQueue: () => void;
  onPrevious: () => void;
  onToggleLoop: () => void;
  onTogglePlayback: () => void;
  onToggleShuffle?: (() => void) | null;
  queueOpen: boolean;
  repeatMode: ZoneRepeatMode;
  shuffle: boolean;
  zonePicker?: ReactNode;
}) {
  const RepeatIcon = repeatMode === "one" ? Repeat1 : Repeat;
  const toggle = (active: boolean) =>
    `grid h-11 w-11 place-items-center rounded-full transition-colors disabled:opacity-35 ${active ? "text-mujin-copper" : "text-mujin-text-secondary active:bg-white/10"}`;
  return (
    <div className="md:hidden">
      <div className="flex items-center justify-between">
        <button
          aria-label={shuffle ? "Shuffle on" : "Shuffle off"}
          aria-pressed={shuffle}
          className={toggle(shuffle)}
          disabled={!onToggleShuffle || live}
          onClick={() => onToggleShuffle?.()}
          type="button"
        >
          <Shuffle className="h-5 w-5" />
        </button>
        <PlaybackControls
          prominent
          isPlaying={isPlaying}
          onNext={onNext}
          onPrevious={onPrevious}
          onToggle={onTogglePlayback}
        />
        <button
          aria-label={repeatModeLabel(repeatMode)}
          aria-pressed={repeatMode !== "off"}
          className={toggle(repeatMode !== "off")}
          disabled={live}
          onClick={onToggleLoop}
          type="button"
        >
          <RepeatIcon className="h-5 w-5" />
        </button>
      </div>
      <div className="mt-2 flex items-center justify-between">
        <div className="flex min-w-11 justify-start">{zonePicker}</div>
        <button
          aria-label="Lyrics"
          aria-pressed={lyricsOpen}
          className={toggle(lyricsOpen)}
          disabled={live}
          onClick={onOpenLyrics}
          type="button"
        >
          <MicVocal className="h-5 w-5" />
        </button>
        <button
          aria-label={queueOpen ? "Back to now playing" : "Queue"}
          aria-pressed={queueOpen}
          className={toggle(queueOpen)}
          onClick={onOpenQueue}
          type="button"
        >
          <ListMusic className="h-5 w-5" />
        </button>
        <div className="flex min-w-11 justify-end">{more}</div>
      </div>
    </div>
  );
}

export const LyricsPanel = memo(function LyricsPanel({
  activeLine,
  activeLineRef,
  backdrop,
  cover,
  onSeek,
  fallback,
  failure,
  instrumental,
  loading,
  onRetry,
  scrollRef,
  source,
  timed,
  title,
}: {
  activeLine: number;
  activeLineRef: RefObject<HTMLElement | null>;
  /** The artist photo; the blurred cover without one. */
  backdrop?: string | null;
  onSeek: (value: number) => void;
  cover: string;
  fallback?: string | null;
  failure: LyricsFailure | null;
  instrumental: boolean;
  loading: boolean;
  onRetry: () => void;
  scrollRef: RefObject<HTMLDivElement | null>;
  source: LyricsSourceKind | null;
  timed: TimedLyric[];
  title: string;
}) {
  return (
    <section
      aria-label="Lyrics"
      className="app-content-shell z-40 overflow-hidden bg-mujin-deep"
    >
      <LyricsBackdrop cover={cover} photo={backdrop} />
      <div className="relative z-2 mx-auto flex h-full max-w-200 flex-col">
        <header className="flex min-w-0 items-center justify-between gap-4 px-6 pb-1 pt-6 sm:px-10">
          <p className="mj-eyebrow truncate">Lyrics · {title}</p>
          {!loading && (timed.length > 0 || fallback) && (
            <LyricsSourceChip source={source} timed={timed.length > 0} />
          )}
        </header>
        <div className="min-h-0 flex-1">
          <LyricsView
            activeLine={activeLine}
            activeLineRef={activeLineRef}
            fallback={fallback}
            failure={failure}
            instrumental={instrumental}
            loading={loading}
            onRetry={onRetry}
            onSeek={onSeek}
            scrollRef={scrollRef}
            size="panel"
            source={source}
            timed={timed}
          />
        </div>
      </div>
    </section>
  );
});
