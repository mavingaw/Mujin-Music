"use client";

import { useArtistAbout } from "@/features/library/credits/use-editorial";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  getApiBaseURL,
  lyricLineEmphasis,
  lyricsEmptyMessage,
  lyricsSourceLabel,
  type LyricsFailure,
  type LyricsSourceKind,
} from "@parson/music-sdk";
import Image from "next/image";
import { memo, type RefObject } from "react";
import type { TimedLyric } from "./player-bar-hooks";

/**
 * The photo behind full-screen lyrics: the artist's picture from the
 * library folder, else their Wikimedia Commons photo when online metadata
 * found one. Null means the blurred cover is used.
 */
export function useLyricsBackdropImage(
  artistId: string | undefined,
  libraryIcon: string | undefined,
  enabled: boolean,
) {
  const about = useArtistAbout(enabled && artistId ? artistId : null);
  const icon = getLibraryImageUrl(libraryIcon, getBaseURL);
  if (icon) return icon;
  const photo = about.data?.image?.url;
  return photo ? `${getApiBaseURL()}${photo}` : null;
}

/** A darkened backdrop: the artist photo, or the cover blurred into colour. */
export function LyricsBackdrop({
  cover,
  photo,
}: {
  cover: string;
  photo?: string | null;
}) {
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 overflow-hidden"
      data-lyrics-backdrop={photo ? "artist-photo" : "blurred-cover"}
    >
      {photo ? (
        <Image
          alt=""
          className="object-cover object-[50%_25%] opacity-55 saturate-[0.7]"
          fill
          sizes="100vw"
          src={photo}
        />
      ) : (
        <div className="absolute -inset-[12%]">
          <Image
            alt=""
            className="scale-110 object-cover opacity-60 blur-[80px] saturate-[1.3]"
            fill
            sizes="40vw"
            src={cover}
          />
        </div>
      )}
      <div className="absolute inset-0 bg-[linear-gradient(to_bottom,rgba(5,5,6,0.72),rgba(5,5,6,0.5)_35%,rgba(5,5,6,0.82))]" />
      <div className="absolute inset-0 bg-[radial-gradient(120%_90%_at_50%_45%,transparent_35%,rgba(5,5,6,0.9)_100%)]" />
    </div>
  );
}

/** "Timed lyrics · LRC embedded in the file". */
export function LyricsSourceChip({
  source,
  timed,
}: {
  source: LyricsSourceKind | null;
  timed: boolean;
}) {
  const where = lyricsSourceLabel(source);
  return (
    <span
      className={`inline-flex max-w-full items-center gap-1.5 truncate rounded-full border px-2.5 py-1 text-[11px] font-medium ${
        timed
          ? "border-mujin-copper/35 bg-mujin-copper/[0.1] text-mujin-copper"
          : "border-white/12 bg-black/20 text-mujin-text-secondary"
      }`}
      data-lyrics-source={source ?? "unknown"}
    >
      <span
        aria-hidden="true"
        className={`h-1.5 w-1.5 shrink-0 rounded-full ${timed ? "bg-mujin-copper" : "bg-mujin-text-tertiary"}`}
      />
      <span className="truncate">
        {timed ? "Timed lyrics" : "Untimed lyrics"}
        {where ? ` · ${where}` : ""}
      </span>
    </span>
  );
}

/** A labelled loading state, never a bare pulsing dot. */
function LyricsLoading() {
  return (
    <div
      aria-live="polite"
      className="grid h-full place-items-center px-6 text-center"
      role="status"
    >
      <div className="flex flex-col items-center">
        <span aria-hidden="true" className="flex h-6 items-end gap-[3px]">
          {[0, 1, 2, 3].map((bar) => (
            <span
              className="w-[3px] origin-bottom rounded-full bg-mujin-copper motion-safe:animate-[mj-lyrics-bar_1.1s_ease-in-out_infinite]"
              key={bar}
              style={{
                animationDelay: `${bar * 140}ms`,
                height: `${10 + ((bar * 7) % 13)}px`,
              }}
            />
          ))}
        </span>
        <p className="mj-title mt-5 text-[24px] text-mujin-text sm:text-[28px]">
          Finding lyrics…
        </p>
        <p className="mt-2 max-w-sm text-[13px] leading-5 text-mujin-text-tertiary">
          Reading the file&apos;s tags and any .lrc beside it, then LRCLIB when
          online lookups are on.
        </p>
      </div>
    </div>
  );
}

function LyricsEmpty({
  failure,
  instrumental,
  onRetry,
}: {
  failure: LyricsFailure | null;
  instrumental: boolean;
  onRetry: () => void;
}) {
  const message = lyricsEmptyMessage(failure, instrumental);
  return (
    <div
      className="grid h-full place-items-center px-6 text-center"
      data-lyrics-state={instrumental ? "instrumental" : (failure ?? "missing")}
      role="status"
    >
      <div className="max-w-md">
        <span
          aria-hidden="true"
          className="mx-auto mb-5 grid h-12 w-12 place-items-center rounded-full border border-white/10 bg-black/25"
        >
          <svg className="h-5 w-5 text-mujin-text-tertiary" viewBox="0 0 20 20">
            <path
              d="M3 6h9M6 10h11M3 14h7"
              fill="none"
              stroke="currentColor"
              strokeLinecap="round"
              strokeWidth="1.6"
            />
            {!instrumental && (
              <path
                d="M3 3l14 14"
                stroke="currentColor"
                strokeLinecap="round"
                strokeWidth="1.6"
              />
            )}
          </svg>
        </span>
        <p className="mj-display text-[34px] leading-tight text-mujin-text sm:text-[44px]">
          {message.title}
        </p>
        <p className="mx-auto mt-3 max-w-sm text-[14px] leading-6 text-mujin-text-secondary">
          {message.detail}
        </p>
        {message.retry && (
          <button
            className="mj-primary mt-6 rounded-full px-5 py-2.5 text-[13px] font-semibold"
            onClick={onRetry}
            type="button"
          >
            Try again
          </button>
        )}
      </div>
    </div>
  );
}

/**
 * Lyrics that follow the music: the sung line lit, the next lines fading
 * with distance, sung lines dimmed, the view scrolling in time. A click on
 * a line seeks to it.
 */
export const LyricsView = memo(function LyricsView({
  activeLine,
  activeLineRef,
  fallback,
  failure,
  instrumental,
  loading,
  onRetry,
  onSeek,
  scrollRef,
  size = "large",
  source,
  timed,
}: {
  activeLine: number;
  activeLineRef: RefObject<HTMLElement | null>;
  fallback?: string | null;
  failure: LyricsFailure | null;
  instrumental: boolean;
  loading: boolean;
  onRetry: () => void;
  onSeek: (value: number) => void;
  scrollRef: RefObject<HTMLDivElement | null>;
  size?: "large" | "panel";
  source: LyricsSourceKind | null;
  timed: TimedLyric[];
}) {
  const lineSize =
    size === "large"
      ? "text-[26px] sm:text-[34px] lg:text-[40px]"
      : "text-[24px] sm:text-[32px]";
  const hasTimed = timed.some((line) => line.text.trim());
  return (
    <div
      ref={scrollRef}
      className="h-full overflow-y-auto overscroll-contain px-6 scrollbar-none sm:px-10 [&::-webkit-scrollbar]:hidden"
      data-lyrics-view={
        loading ? "loading" : hasTimed ? "timed" : fallback ? "plain" : "empty"
      }
      style={
        hasTimed || fallback
          ? {
              maskImage:
                "linear-gradient(to bottom, transparent 0, black 14%, black 78%, transparent 100%)",
              WebkitMaskImage:
                "linear-gradient(to bottom, transparent 0, black 14%, black 78%, transparent 100%)",
            }
          : undefined
      }
    >
      {loading ? (
        <LyricsLoading />
      ) : hasTimed ? (
        <ol
          aria-label="Timed lyrics"
          className="space-y-[0.55em] pb-[55vh] pt-[30vh]"
        >
          {timed.map((line, index) => {
            const emphasis = lyricLineEmphasis(index, activeLine);
            const active = emphasis.state === "active";
            const setRef = (node: HTMLElement | null) => {
              if (index === activeLine) activeLineRef.current = node;
            };
            if (!line.text.trim()) {
              // An instrumental break: three dots while it lasts.
              return (
                <li
                  aria-hidden={!active}
                  className={`flex h-[1.2em] items-center gap-2 ${lineSize}`}
                  data-lyric-state={emphasis.state}
                  key={`${line.time}-${index}`}
                  ref={setRef}
                >
                  {active &&
                    [0, 1, 2].map((dot) => (
                      <span
                        className="h-2 w-2 rounded-full bg-mujin-text/80 motion-safe:animate-pulse"
                        key={dot}
                        style={{ animationDelay: `${dot * 200}ms` }}
                      />
                    ))}
                  {active && <span className="sr-only">Instrumental</span>}
                </li>
              );
            }
            return (
              <li
                aria-current={active ? "true" : undefined}
                data-lyric-state={emphasis.state}
                key={`${line.time}-${index}`}
                ref={setRef}
              >
                <button
                  className={`mj-title block w-full origin-left cursor-pointer text-left leading-[1.22] transition-[opacity,color,transform] duration-500 ease-out hover:opacity-100! ${lineSize} ${
                    active
                      ? "text-mujin-text motion-safe:scale-[1.02] [text-shadow:0_0_32px_rgba(242,238,230,0.18)]"
                      : "text-mujin-text"
                  }`}
                  onClick={() => onSeek(line.time)}
                  style={{ opacity: emphasis.opacity }}
                  title="Play from this line"
                  type="button"
                >
                  {line.text}
                </button>
              </li>
            );
          })}
        </ol>
      ) : fallback ? (
        <p className="mj-title whitespace-pre-line py-[18vh] text-[24px] leading-[1.55] text-mujin-text/85 sm:text-[30px]">
          {fallback}
        </p>
      ) : (
        <LyricsEmpty
          failure={failure}
          instrumental={instrumental}
          onRetry={onRetry}
        />
      )}
    </div>
  );
});
