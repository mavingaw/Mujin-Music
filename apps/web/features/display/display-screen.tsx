"use client";

import ParsonBrandMark from "@/components/icons/parson-brand-mark";
import MujinLockup from "@/components/icons/mujin-lockup";
import {
  rgbString,
  type ArtworkPalette,
} from "@/features/player/artwork-palette";
import { useArtworkPalette } from "@/features/player/use-artwork-palette";
import {
  activeLyricIndex,
  displayAssetURL,
  displayCaption,
  displayDrift,
  displayQualityChip,
  displayTrackLine,
  formatDisplayTime,
  lyricLineEmphasis,
  waveformBars,
  type DisplayQualityChip,
  type DisplayQueued,
  type DisplayState,
  type DisplayTrack,
  type DisplayWaveform,
  type TimedLyricLine,
} from "@parson/music-sdk";
import { Expand, MicVocal, Minimize, Pause, Unlink } from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  clockText,
  displayMode,
  lyricsWindow,
  pairingCountdown,
  waveformBarCount,
  type DisplayMode,
} from "./display-model";
import {
  useDisplayKey,
  useDisplayLyrics,
  useDisplayPairing,
  useDisplayPosition,
  useDisplayState,
  useDisplayWaveform,
  useMinuteClock,
  usePointerActivity,
  useReducedMotion,
  useWakeLock,
} from "./use-display";

const LIGHT_COLOR: Record<DisplayQualityChip["light"], string> = {
  lossless: "var(--color-quality-lossless)",
  enhanced: "var(--color-quality-enhanced)",
  high: "var(--color-quality-high)",
  low: "var(--color-quality-low)",
  unknown: "var(--color-mujin-text-tertiary)",
};

/**
 * Mujin Music on a TV, tablet or spare browser: what one zone plays, full
 * screen, without controls. A large artist photo (or the blurred cover),
 * the cover, title and artist, the release facts, the quality chip with
 * its signal light, a waveform seek bar and, when the track has them, its
 * timed lyrics in large type.
 */
export default function DisplayScreen() {
  const { key, ready, setKey } = useDisplayKey();
  const { offline, receivedAt, state } = useDisplayState(key, () =>
    setKey(null),
  );
  const { error: pairingError, pairing } = useDisplayPairing(
    ready && !key,
    setKey,
  );
  const [lyricsHidden, setLyricsHidden] = useState(false);
  const track = state?.status === "playing" ? state.track : null;
  const songId = track?.song_id ?? null;
  const lyricsAllowed =
    Boolean(track) && !track?.live && state?.display.lyrics !== "off";
  const lyrics = useDisplayLyrics(key, songId, lyricsAllowed);
  const waveform = useDisplayWaveform(
    key,
    songId,
    Boolean(track && !track.live),
  );
  const position = useDisplayPosition(state, receivedAt);
  const now = useMinuteClock();
  const pointer = usePointerActivity();
  const reducedMotion = useReducedMotion();
  const [fullscreen, setFullscreen] = useState(false);
  useWakeLock(Boolean(key));

  useEffect(() => {
    const onChange = () => setFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);
  const toggleFullscreen = () => {
    if (document.fullscreenElement)
      void document.exitFullscreen().catch(() => {});
    else void document.documentElement.requestFullscreen?.().catch(() => {});
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "f" || event.key === "F") toggleFullscreen();
      if (event.key === "l" || event.key === "L")
        setLyricsHidden((hidden) => !hidden);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const mode = displayMode({
    hasKey: Boolean(key),
    state,
    timedLines: lyrics.timed,
    lyricsHidden,
  });
  const drift = useDrift(reducedMotion);

  if (!ready) return <main className="fixed inset-0 bg-mujin-deep" />;
  if (mode === "pairing") {
    return (
      <PairingScreen
        code={pairing?.code ?? null}
        error={pairingError}
        expiresAt={pairing?.expiresAt ?? 0}
      />
    );
  }

  return (
    <main
      aria-label="Now playing display"
      className={`fixed inset-0 overflow-hidden bg-mujin-deep text-mujin-text ${pointer ? "" : "mj-display-hide-cursor"}`}
    >
      {track ? (
        <Backdrop lyrics={mode === "lyrics"} track={track} />
      ) : (
        <IdleBackdrop />
      )}
      <div
        className="mj-display-drift relative flex h-full flex-col px-[clamp(20px,4.6vw,96px)] pb-[clamp(18px,4.4vh,56px)] pt-[clamp(16px,3.8vh,48px)]"
        style={{ transform: `translate3d(${drift.x}px, ${drift.y}px, 0)` }}
      >
        <TopBar
          credit={track?.artist_image_url ? track.artist_image_credit : null}
          now={now}
          showClock={Boolean(track)}
          state={state}
        />
        {mode === "connecting" ? (
          <CenterMessage
            caption="Connecting to Mujin Music…"
            now={now}
            title={null}
          />
        ) : mode === "idle" || mode === "missing" ? (
          <CenterMessage
            caption={displayCaption(state)}
            now={now}
            title={state?.display.name ?? null}
          />
        ) : track ? (
          <NowPlaying
            lines={lyrics.timed}
            mode={mode}
            position={position}
            reducedMotion={reducedMotion}
            state={state!}
            track={track}
            waveform={waveform}
          />
        ) : null}
      </div>
      {offline && (
        <p
          className="absolute left-1/2 top-[clamp(16px,3.8vh,48px)] -translate-x-1/2 rounded-full border border-white/10 bg-black/50 px-4 py-1.5 text-[clamp(12px,1.5vh,16px)] text-mujin-text-secondary backdrop-blur-md"
          role="status"
        >
          Reconnecting to Mujin Music…
        </p>
      )}
      <Toolbar
        fullscreen={fullscreen}
        lyricsAvailable={lyrics.timed.length > 0 && lyricsAllowed}
        lyricsHidden={lyricsHidden}
        onForget={() => setKey(null)}
        onToggleFullscreen={toggleFullscreen}
        onToggleLyrics={() => setLyricsHidden((hidden) => !hidden)}
        visible={pointer}
      />
    </main>
  );
}

/** The layout's slow anti-burn-in drift, recomputed every minute. */
function useDrift(reducedMotion: boolean) {
  const [at, setAt] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setAt(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);
  return displayDrift(at, { reducedMotion });
}

function Backdrop({ lyrics, track }: { lyrics: boolean; track: DisplayTrack }) {
  const cover = displayAssetURL(track.cover_url);
  const photo = displayAssetURL(track.artist_image_url);
  const palette = useArtworkPalette(cover);
  const [photoFailed, setPhotoFailed] = useState<string | null>(null);
  const showPhoto = Boolean(photo) && photoFailed !== photo;
  return (
    <div
      aria-hidden="true"
      className="absolute inset-0"
      style={{ backgroundColor: rgbString(palette.deep) }}
    >
      {cover && (
        <div
          className="mj-display-layer absolute -inset-[14%]"
          key={`cover-${cover}`}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            alt=""
            className="h-full w-full scale-110 object-cover opacity-60 blur-[90px] saturate-[1.35]"
            src={cover}
          />
        </div>
      )}
      <div
        className="absolute inset-0"
        style={{
          background: `radial-gradient(65% 70% at 18% 78%, ${rgbString(palette.glow, 0.3)}, transparent 70%), radial-gradient(90% 80% at 85% 20%, ${rgbString(palette.glow, 0.12)}, transparent 70%)`,
        }}
      />
      {showPhoto && (
        <div
          className="mj-display-layer mj-display-photo absolute inset-y-0 right-0 w-[min(100%,max(62%,calc(100vh*1.1)))] overflow-hidden"
          key={`photo-${photo}`}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            alt=""
            className="mj-display-kenburns h-full w-full object-cover object-[70%_25%] [filter:saturate(0.9)_contrast(1.04)]"
            onError={() => setPhotoFailed(photo)}
            src={photo!}
          />
          <div
            className="absolute inset-0 mix-blend-soft-light"
            style={{ backgroundColor: rgbString(palette.glow, 0.35) }}
          />
        </div>
      )}
      <div
        className={`absolute inset-0 transition-colors duration-700 ${lyrics ? "bg-black/55" : "bg-black/20"}`}
      />
      <div className="absolute inset-x-0 bottom-0 h-[62%] bg-gradient-to-t from-[rgb(5_5_6/0.92)] via-[rgb(5_5_6/0.55)] to-transparent" />
      <div className="absolute inset-x-0 top-0 h-[22%] bg-gradient-to-b from-[rgb(5_5_6/0.6)] to-transparent" />
      <div className="absolute inset-0 bg-[radial-gradient(150%_110%_at_50%_45%,transparent_55%,rgb(5_5_6/0.75)_100%)]" />
    </div>
  );
}

function IdleBackdrop() {
  return (
    <div aria-hidden="true" className="absolute inset-0 bg-mujin-deep">
      <div className="absolute inset-0 bg-[radial-gradient(60%_55%_at_50%_42%,rgb(217_139_95/0.08),transparent_70%)]" />
      <div className="absolute inset-0 bg-[radial-gradient(150%_110%_at_50%_45%,transparent_50%,rgb(0_0_0/0.8)_100%)]" />
    </div>
  );
}

function TopBar({
  credit,
  now,
  showClock,
  state,
}: {
  credit: string | null | undefined;
  now: Date;
  /** Off when the big centred clock already shows the time. */
  showClock: boolean;
  state: DisplayState | null;
}) {
  const zone = state?.status === "playing" ? state.zone : null;
  const paused = zone && zone.status !== "playing";
  return (
    <header className="relative z-10 flex items-start justify-between gap-6">
      <div className="flex min-w-0 items-center gap-[clamp(10px,1.2vw,18px)]">
        <ParsonBrandMark className="h-[clamp(22px,3.2vh,40px)] w-[clamp(22px,3.2vh,40px)] shrink-0" />
        {zone && (
          <div className="flex min-w-0 items-center gap-3">
            <span className="mj-eyebrow truncate text-[clamp(11px,1.45vh,16px)]! text-mujin-text-secondary!">
              {zone.name}
            </span>
            {paused && (
              <span className="flex items-center gap-1.5 rounded-full border border-white/12 bg-black/30 px-2.5 py-0.5 text-[clamp(11px,1.35vh,15px)] text-mujin-text-secondary backdrop-blur-md">
                <Pause aria-hidden="true" className="h-3 w-3" />
                Paused
              </span>
            )}
          </div>
        )}
      </div>
      <div className="shrink-0 text-right">
        {showClock && (
          <p className="mj-display mj-tnum text-[clamp(26px,4.2vh,56px)] leading-none text-mujin-text">
            {clockText(now)}
          </p>
        )}
        {credit && (
          <p className="ml-auto mt-2 w-fit max-w-[min(42vw,520px)] truncate rounded-full bg-black/45 px-2.5 py-0.5 text-[clamp(10px,1.15vh,13px)] text-mujin-text-secondary backdrop-blur-md">
            {credit}
          </p>
        )}
      </div>
    </header>
  );
}

function CenterMessage({
  caption,
  now,
  title,
}: {
  caption: string;
  now: Date;
  title: string | null;
}) {
  return (
    <section className="flex min-h-0 flex-1 flex-col items-center justify-center text-center">
      <div className="mj-display-rise flex flex-col items-center">
        <p className="mj-display mj-tnum text-[clamp(72px,17vh,220px)] leading-none text-mujin-text">
          {clockText(now)}
        </p>
        <p className="mt-[3vh] max-w-[60vw] text-[clamp(16px,2.4vh,28px)] text-mujin-text-secondary">
          {caption}
        </p>
        {title && (
          <p className="mj-eyebrow mt-[1.6vh] text-[clamp(11px,1.4vh,15px)]!">
            {title}
          </p>
        )}
      </div>
    </section>
  );
}

function NowPlaying({
  lines,
  mode,
  position,
  reducedMotion,
  state,
  track,
  waveform,
}: {
  lines: TimedLyricLine[];
  mode: DisplayMode;
  position: number;
  reducedMotion: boolean;
  state: DisplayState;
  track: DisplayTrack;
  waveform: DisplayWaveform | null;
}) {
  const cover = displayAssetURL(track.cover_url);
  const palette = useArtworkPalette(cover);
  const chip = displayQualityChip(track, state.zone);
  const lyrics = mode === "lyrics";
  const trackLine = displayTrackLine(track);
  const facts = [
    track.album,
    track.composer ? `Composed by ${track.composer}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const next = state.up_next[0] ?? null;
  return (
    <>
      <section className="relative flex min-h-0 flex-1 flex-col justify-center py-[3vh]">
        {lyrics ? (
          <DisplayLyrics
            lines={lines}
            position={position}
            reducedMotion={reducedMotion}
          />
        ) : track.biography ? (
          <Biography artist={track.artist} biography={track.biography} />
        ) : null}
      </section>
      <section
        className="mj-display-rise flex items-end gap-[clamp(16px,2.6vw,56px)]"
        key={track.song_id ?? track.title}
      >
        <Cover
          cover={cover}
          lyrics={lyrics}
          palette={palette}
          title={track.title}
        />
        <div className="min-w-0 flex-1 pb-[0.4vh]">
          {trackLine && (
            <p className="mj-eyebrow mb-[1.2vh] text-[clamp(11px,1.45vh,16px)]!">
              {track.live ? "Live radio" : trackLine}
            </p>
          )}
          <h1
            className={`mj-display line-clamp-2 text-mujin-text [text-wrap:balance] ${lyrics ? "text-[clamp(26px,min(5vh,7vw),64px)]" : "text-[clamp(34px,min(8.6vh,9vw),120px)]"}`}
            title={track.title}
          >
            {track.title}
          </h1>
          <p
            className={`mt-[1vh] truncate text-mujin-text-secondary ${lyrics ? "text-[clamp(16px,min(2.6vh,4vw),30px)]" : "text-[clamp(18px,min(3.6vh,5vw),44px)]"}`}
          >
            {track.artist}
          </p>
          {facts && !lyrics && (
            <p className="mt-[0.8vh] truncate text-[clamp(14px,2vh,24px)] text-mujin-text-tertiary">
              {facts}
            </p>
          )}
          {chip && (
            <div className="mt-[2vh]">
              <QualityChip chip={chip} large={!lyrics} />
            </div>
          )}
        </div>
        {next && !lyrics && <UpNext item={next} />}
      </section>
      <SeekBar
        duration={track.duration_ms || state.zone?.duration_ms || 0}
        live={track.live}
        position={position}
        waveform={waveform}
      />
    </>
  );
}

function Cover({
  cover,
  lyrics,
  palette,
  title,
}: {
  cover: string | null;
  lyrics: boolean;
  palette: ArtworkPalette;
  title: string;
}) {
  return (
    <div
      className={`relative aspect-square shrink-0 overflow-hidden rounded-[clamp(4px,0.6vh,8px)] bg-mujin-surface-1 transition-[width] duration-700 ${lyrics ? "w-[clamp(64px,min(16vh,18vw),200px)]" : "w-[clamp(96px,min(34vh,28vw),420px)]"}`}
      style={{
        boxShadow: `0 0 0 1px rgb(242 238 230 / 0.08), 0 40px 100px -40px ${rgbString(palette.glow, 0.6)}, 0 24px 60px -30px rgb(0 0 0 / 0.95)`,
      }}
    >
      {cover ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img alt={title} className="h-full w-full object-cover" src={cover} />
      ) : (
        <div className="grid h-full w-full place-items-center">
          <ParsonBrandMark className="h-1/3 w-1/3 opacity-40" />
        </div>
      )}
    </div>
  );
}

export function QualityChip({
  chip,
  large = false,
}: {
  chip: DisplayQualityChip;
  large?: boolean;
}) {
  const parts = [chip.format, chip.tier].filter(Boolean) as string[];
  return (
    <span
      className={`inline-flex max-w-full items-center gap-[0.7em] rounded-full border border-white/12 bg-black/35 font-mono backdrop-blur-md ${large ? "px-[1em] py-[0.45em] text-[clamp(12px,1.7vh,20px)]" : "px-[0.9em] py-[0.4em] text-[clamp(11px,1.45vh,16px)]"}`}
    >
      <span
        aria-hidden="true"
        className="h-[0.62em] w-[0.62em] shrink-0 rounded-full"
        style={{
          backgroundColor: LIGHT_COLOR[chip.light],
          boxShadow: `0 0 0 0.28em color-mix(in srgb, ${LIGHT_COLOR[chip.light]} 20%, transparent)`,
        }}
      />
      <span className="truncate text-mujin-text">{parts.join(" · ")}</span>
      {chip.path && (
        <span
          className="shrink-0 font-sans text-[0.92em] font-medium"
          style={{ color: LIGHT_COLOR[chip.light] }}
        >
          {chip.path}
        </span>
      )}
    </span>
  );
}

function UpNext({ item }: { item: DisplayQueued }) {
  const cover = displayAssetURL(item.cover_url);
  return (
    <aside className="hidden w-[clamp(220px,20vw,360px)] shrink-0 items-center gap-[0.9vw] self-end rounded-[clamp(6px,0.9vh,12px)] border border-white/[0.08] bg-black/30 p-[clamp(8px,1vh,14px)] backdrop-blur-xl lg:flex">
      <div className="relative aspect-square w-[clamp(40px,5.6vh,68px)] shrink-0 overflow-hidden rounded-[4px] bg-mujin-surface-2">
        {cover && (
          // eslint-disable-next-line @next/next/no-img-element
          <img alt="" className="h-full w-full object-cover" src={cover} />
        )}
      </div>
      <div className="min-w-0">
        <p className="mj-eyebrow text-[clamp(9px,1.15vh,12px)]!">Up next</p>
        <p className="mt-0.5 truncate text-[clamp(13px,1.7vh,19px)] font-medium text-mujin-text">
          {item.title}
        </p>
        <p className="truncate text-[clamp(12px,1.5vh,16px)] text-mujin-text-tertiary">
          {item.artist}
        </p>
      </div>
    </aside>
  );
}

function Biography({
  artist,
  biography,
}: {
  artist: string;
  biography: NonNullable<DisplayTrack["biography"]>;
}) {
  return (
    <article
      aria-label={`About ${artist}`}
      className="mj-display-rise max-w-full self-start landscape:max-w-[min(46vw,760px)]"
    >
      <p className="mj-eyebrow text-[clamp(11px,1.4vh,15px)]!">
        About {artist}
      </p>
      <p className="mt-[1.4vh] line-clamp-5 text-[clamp(15px,2.25vh,27px)] leading-[1.55] text-mujin-text-secondary [text-wrap:pretty]">
        {biography.text}
      </p>
      <p className="mt-[1.2vh] text-[clamp(10px,1.2vh,13px)] text-mujin-text-tertiary">
        From {biography.source} · {biography.license}
      </p>
    </article>
  );
}

/**
 * Timed lyrics in large type. The sung line sits a little above centre at
 * full strength; the lines around it fade with distance; the column glides
 * to each new line.
 */
function DisplayLyrics({
  lines,
  position,
  reducedMotion,
}: {
  lines: TimedLyricLine[];
  position: number;
  reducedMotion: boolean;
}) {
  const active = activeLyricIndex(lines, position / 1000);
  const { start, end } = lyricsWindow(lines.length, active);
  const containerRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLParagraphElement>(null);
  const [offset, setOffset] = useState(0);
  useLayoutEffect(() => {
    const container = containerRef.current;
    const line = activeRef.current;
    if (!container) return;
    const anchor = container.parentElement
      ? container.parentElement.clientHeight * 0.42
      : 0;
    const top = line ? line.offsetTop + line.offsetHeight / 2 : 0;
    setOffset(anchor - top);
  }, [active, start, lines]);
  return (
    <div
      aria-live="off"
      className="relative h-full min-h-0 overflow-hidden [mask-image:linear-gradient(to_bottom,transparent,#000_18%,#000_78%,transparent)]"
    >
      <div
        className="mj-display-lyrics absolute inset-x-0 top-0 max-w-[min(78vw,1500px)]"
        ref={containerRef}
        style={{
          transform: `translate3d(0, ${offset}px, 0)`,
          transition: reducedMotion ? "none" : undefined,
        }}
      >
        {lines.slice(start, end).map((line, offsetIndex) => {
          const index = start + offsetIndex;
          const emphasis = lyricLineEmphasis(index, active);
          const isActive = emphasis.state === "active";
          return (
            <p
              className={`mj-display py-[0.9vh] text-[clamp(26px,min(6.4vh,7.4vw),88px)] leading-[1.12] [text-wrap:balance] ${isActive ? "text-mujin-text" : "text-mujin-text-secondary"}`}
              key={`${index}-${line.time}`}
              ref={isActive ? activeRef : undefined}
              style={{
                opacity: emphasis.opacity,
                transform: isActive ? "scale(1)" : "scale(0.94)",
              }}
            >
              {line.text || "•  •  •"}
            </p>
          );
        })}
      </div>
    </div>
  );
}

function SeekBar({
  duration,
  live,
  position,
  waveform,
}: {
  duration: number;
  live: boolean;
  position: number;
  waveform: DisplayWaveform | null;
}) {
  const barRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(1200);
  useEffect(() => {
    const element = barRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(entry.contentRect.width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const bars = useMemo(
    () => waveformBars(waveform, waveformBarCount(width)),
    [waveform, width],
  );
  const progress = duration > 0 ? Math.min(1, position / duration) : 0;
  if (live) {
    return (
      <div className="mt-[3.2vh] flex items-center gap-3 text-[clamp(12px,1.6vh,18px)] text-mujin-text-secondary">
        <span className="mj-display-breathe h-2 w-2 rounded-full bg-quality-clipping" />
        <span className="mj-eyebrow text-[clamp(11px,1.4vh,15px)]!">Live</span>
        <span className="h-px flex-1 bg-white/15" />
      </div>
    );
  }
  return (
    <div
      aria-label={`${formatDisplayTime(position)} of ${formatDisplayTime(duration)}`}
      className="mt-[3.2vh] grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-[clamp(12px,1.4vw,24px)]"
      role="img"
    >
      <span className="mj-tnum w-[4.5ch] font-mono text-[clamp(12px,1.6vh,19px)] text-mujin-text-secondary">
        {formatDisplayTime(position)}
      </span>
      <div className="relative h-[clamp(28px,4.6vh,56px)]" ref={barRef}>
        {bars.length ? (
          <div className="absolute inset-0 flex items-center gap-[2px]">
            {bars.map((level, index) => {
              const played = (index + 0.5) / bars.length <= progress;
              return (
                <span
                  className={`min-w-0 flex-1 rounded-full transition-colors duration-300 ${played ? "bg-mujin-text" : "bg-white/18"}`}
                  key={index}
                  style={{ height: `${Math.max(8, level * 100)}%` }}
                />
              );
            })}
          </div>
        ) : (
          <div className="absolute inset-x-0 top-1/2 h-[3px] -translate-y-1/2 rounded-full bg-white/15">
            <div
              className="h-full rounded-full bg-mujin-text"
              style={{ width: `${progress * 100}%` }}
            />
          </div>
        )}
        <span
          aria-hidden="true"
          className="absolute top-1/2 h-[clamp(8px,1.1vh,13px)] w-[clamp(8px,1.1vh,13px)] -translate-x-1/2 -translate-y-1/2 rounded-full bg-mujin-copper shadow-[0_0_0_4px_rgb(217_139_95/0.22)]"
          style={{ left: `${progress * 100}%` }}
        />
      </div>
      <span className="mj-tnum w-[4.5ch] text-right font-mono text-[clamp(12px,1.6vh,19px)] text-mujin-text-tertiary">
        {formatDisplayTime(duration)}
      </span>
    </div>
  );
}

function Toolbar({
  fullscreen,
  lyricsAvailable,
  lyricsHidden,
  onForget,
  onToggleFullscreen,
  onToggleLyrics,
  visible,
}: {
  fullscreen: boolean;
  lyricsAvailable: boolean;
  lyricsHidden: boolean;
  onForget: () => void;
  onToggleFullscreen: () => void;
  onToggleLyrics: () => void;
  visible: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  return (
    <nav
      aria-label="Display options"
      className={`absolute bottom-[clamp(84px,13vh,150px)] right-[clamp(20px,4.6vw,96px)] flex items-center gap-1 rounded-full border border-white/10 bg-black/55 p-1 backdrop-blur-xl transition-opacity duration-300 ${visible ? "opacity-100" : "pointer-events-none opacity-0"}`}
    >
      {lyricsAvailable && (
        <ToolbarButton
          active={!lyricsHidden}
          label={lyricsHidden ? "Show lyrics (L)" : "Hide lyrics (L)"}
          onClick={onToggleLyrics}
        >
          <MicVocal className="h-4 w-4" />
        </ToolbarButton>
      )}
      <ToolbarButton
        active={false}
        label={fullscreen ? "Exit full screen (F)" : "Full screen (F)"}
        onClick={onToggleFullscreen}
      >
        {fullscreen ? (
          <Minimize className="h-4 w-4" />
        ) : (
          <Expand className="h-4 w-4" />
        )}
      </ToolbarButton>
      {confirming ? (
        <button
          className="rounded-full bg-quality-clipping/15 px-3 py-1.5 text-xs font-medium text-mujin-text hover:bg-quality-clipping/25"
          onClick={onForget}
          type="button"
        >
          Forget this screen
        </button>
      ) : (
        <ToolbarButton
          active={false}
          label="Unpair this screen"
          onClick={() => setConfirming(true)}
        >
          <Unlink className="h-4 w-4" />
        </ToolbarButton>
      )}
    </nav>
  );
}

function ToolbarButton({
  active,
  children,
  label,
  onClick,
}: {
  active: boolean;
  children: ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      aria-pressed={active}
      className={`grid h-9 w-9 place-items-center rounded-full transition-colors ${active ? "bg-mujin-copper/[0.18] text-mujin-copper" : "text-mujin-text-secondary hover:bg-white/10 hover:text-mujin-text"}`}
      onClick={onClick}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}

function PairingScreen({
  code,
  error,
  expiresAt,
}: {
  code: string | null;
  error: string | null;
  expiresAt: number;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, []);
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return (
    <main className="fixed inset-0 overflow-hidden bg-mujin-deep text-mujin-text">
      <IdleBackdrop />
      <div className="relative flex h-full flex-col items-center justify-center px-6 text-center">
        <MujinLockup className="h-[clamp(28px,5vh,56px)] w-auto" />
        <p className="mj-eyebrow mt-[7vh] text-[clamp(11px,1.5vh,16px)]!">
          Display
        </p>
        <h1 className="mj-display mt-[1.4vh] text-[clamp(30px,5.4vh,64px)]">
          Pair this screen
        </h1>
        <div
          aria-label={
            code ? `Pairing code ${code.split("").join(" ")}` : "Getting a code"
          }
          className="mt-[5vh] flex gap-[clamp(6px,1vw,16px)]"
          role="group"
        >
          {(code ?? "······").split("").map((digit, index) => (
            <span
              className={`grid h-[clamp(64px,13vh,150px)] w-[clamp(46px,9vh,108px)] place-items-center rounded-[clamp(8px,1.2vh,14px)] border border-white/12 bg-white/[0.04] font-mono text-[clamp(36px,8.6vh,100px)] font-light text-mujin-text ${index === 3 ? "ml-[clamp(8px,1.4vw,22px)]" : ""} ${code ? "" : "mj-display-breathe text-mujin-text-tertiary"}`}
              key={index}
            >
              {digit}
            </span>
          ))}
        </div>
        <p className="mt-[5vh] max-w-[min(90vw,760px)] text-[clamp(15px,2.3vh,26px)] leading-relaxed text-mujin-text-secondary">
          On your phone or computer, open Mujin Music, go to{" "}
          <span className="text-mujin-text">Settings → Displays</span> and enter
          this code. The screen then follows the zone you choose.
        </p>
        <p className="mt-[2vh] text-[clamp(12px,1.6vh,18px)] text-mujin-text-tertiary">
          {error ??
            (code
              ? `New code in ${pairingCountdown(expiresAt, now)} · ${origin.replace(/^https?:\/\//, "")}/display`
              : "Getting a code…")}
        </p>
      </div>
    </main>
  );
}
