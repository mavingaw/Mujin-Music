"use client";

import Image from "next/image";
import Link from "next/link";
import { useRef, type ReactNode } from "react";
import { ChevronLeft, ChevronRight, Radio } from "lucide-react";
import {
  albumQualityLabel,
  type HomeAlbum,
  type HomeArtist,
  type HomeBestYears,
  type HomeDailyPick,
  type HomeGenre,
  type LiveStation,
} from "@parson/music-sdk";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import AlbumCard from "@/features/library/album-card";
import { generatedCover } from "@/features/library/artwork/generated-cover";
import { usePlayAlbum } from "@/features/library/use-play-album";
import { StationArtwork } from "@/features/live-radio/station-ui";
import { usePlayStation } from "@/features/live-radio/use-live-radio";
import { stationHref } from "@/features/live-radio/live-radio-model";
import PlayIcon from "@/components/icons/play";
import { cn } from "@/lib/cn";

export function coverUrl(cover: string, title: string, subtitle = "") {
  return (
    getLibraryImageUrl(cover, getBaseURL) ?? generatedCover(title, subtitle)
  );
}

/** A section title with an optional eyebrow, "See all" link and arrows. */
export function SectionHeader({
  action,
  eyebrow,
  onScroll,
  title,
}: {
  action?: { href: string; label: string };
  eyebrow?: string;
  onScroll?: (direction: "left" | "right") => void;
  title: string;
}) {
  return (
    <div className="mb-4 flex items-end justify-between gap-4 border-b border-white/[0.06] pb-3">
      <div className="min-w-0">
        {eyebrow && <p className="mj-eyebrow mb-1.5">{eyebrow}</p>}
        <h2 className="mj-title truncate text-[22px] leading-tight text-mujin-text md:text-[24px]">
          {title}
        </h2>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        {action && (
          <Link
            className="mr-1 rounded-full px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-mujin-text-tertiary transition-colors hover:bg-white/[0.05] hover:text-mujin-text"
            href={action.href}
          >
            {action.label}
          </Link>
        )}
        {onScroll && (
          <>
            <ScrollButton direction="left" onClick={onScroll} title={title} />
            <ScrollButton direction="right" onClick={onScroll} title={title} />
          </>
        )}
      </div>
    </div>
  );
}

function ScrollButton({
  direction,
  onClick,
  title,
}: {
  direction: "left" | "right";
  onClick: (direction: "left" | "right") => void;
  title: string;
}) {
  const Icon = direction === "left" ? ChevronLeft : ChevronRight;
  return (
    <button
      aria-label={`Scroll ${title} ${direction}`}
      className="hidden h-8 w-8 items-center justify-center rounded-full border border-white/[0.08] text-mujin-text-secondary transition-colors hover:border-white/20 hover:text-mujin-text sm:flex"
      onClick={() => onClick(direction)}
      type="button"
    >
      <Icon className="h-4 w-4" />
    </button>
  );
}

/** A horizontally scrolling row of cards with a section header. */
export function ShelfRow({
  action,
  children,
  eyebrow,
  size = "md",
  title,
}: {
  action?: { href: string; label: string };
  children: ReactNode;
  eyebrow?: string;
  /** `sm` fits more, smaller cards (artists, genres). */
  size?: "sm" | "md";
  title: string;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const scroll = (direction: "left" | "right") => {
    const node = scrollRef.current;
    if (!node) return;
    const amount = Math.min(900, node.clientWidth * 0.85);
    node.scrollBy({
      left: direction === "left" ? -amount : amount,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth",
    });
  };
  return (
    <section aria-label={title}>
      <SectionHeader
        action={action}
        eyebrow={eyebrow}
        onScroll={scroll}
        title={title}
      />
      <div
        ref={scrollRef}
        className={cn(
          "-mx-1 grid snap-x snap-mandatory grid-flow-col gap-4 overflow-x-auto overflow-y-visible px-1 pb-3 pt-1 scrollbar-none md:gap-5 [&::-webkit-scrollbar]:hidden",
          size === "md"
            ? "auto-cols-[calc((100%-16px)/2.25)] sm:auto-cols-[calc((100%-48px)/3.4)] lg:auto-cols-[calc((100%-80px)/5)] 2xl:auto-cols-[calc((100%-100px)/6)]"
            : "auto-cols-[calc((100%-32px)/3.2)] sm:auto-cols-[calc((100%-80px)/5)] lg:auto-cols-[calc((100%-120px)/7)]",
        )}
      >
        {children}
      </div>
    </section>
  );
}

export function HomeAlbumCard({ album }: { album: HomeAlbum }) {
  const label = albumQualityLabel(album.quality);
  return (
    <div className="min-w-0 snap-start">
      <AlbumCard
        album_cover={album.cover_url}
        album_id={album.id}
        album_name={album.name}
        artist_id={album.artist.id}
        artist_name={album.artist.name}
        first_release_date={album.first_release_date}
        formatLabel={label}
        hiRes={Boolean(album.quality?.hi_res)}
        lossless={Boolean(album.quality?.lossless)}
        showYear
      />
    </div>
  );
}

/** Round portraits, Roon-style, with the reason they are here. */
export function ArtistCircle({ artist }: { artist: HomeArtist }) {
  const image = coverUrl(artist.image_url, artist.name);
  return (
    <Link
      className="group block min-w-0 snap-start text-center"
      href={`/artist?id=${encodeURIComponent(artist.id)}`}
    >
      <span className="relative mx-auto block aspect-square w-full overflow-hidden rounded-full bg-mujin-surface-1 shadow-[0_0_0_1px_rgba(242,238,230,0.07),0_18px_40px_-20px_rgba(0,0,0,0.9)]">
        <Image
          alt=""
          className="object-cover grayscale-[0.25] transition-[transform,filter] duration-500 ease-out group-hover:grayscale-0 motion-safe:group-hover:scale-[1.04]"
          fill
          sizes="180px"
          src={image}
        />
      </span>
      <span className="mt-3 block truncate text-[13.5px] font-medium text-mujin-text group-hover:underline">
        {artist.name}
      </span>
      <span className="block truncate text-[12px] text-mujin-text-tertiary">
        {artist.reason ??
          `${artist.album_count} ${artist.album_count === 1 ? "album" : "albums"}`}
      </span>
    </Link>
  );
}

/**
 * Daily Picks: a wide band on a blurred-art backdrop of the first pick, the
 * date as an eyebrow, the reason line, and the picks as a cover collage.
 */
export function DailyPicksBand({
  date,
  picks,
}: {
  date: string;
  picks: HomeDailyPick[];
}) {
  const playAlbum = usePlayAlbum();
  if (!picks.length) return null;
  const lead = picks[0]!;
  const backdrop = coverUrl(
    lead.album.cover_url,
    lead.album.name,
    lead.album.artist.name,
  );
  const day = formatPickDate(date);
  const artists = picks.map((pick) => pick.album.artist.name).join(" / ");
  return (
    <section
      aria-label="Daily Picks"
      className="relative isolate overflow-hidden rounded-xl border border-white/[0.07] bg-mujin-surface-1"
    >
      <div
        aria-hidden="true"
        className="mj-art-backdrop -z-10 opacity-60!"
        style={{ backgroundImage: `url("${backdrop}")` }}
      />
      <div
        aria-hidden="true"
        className="absolute inset-0 -z-10 bg-linear-to-r from-mujin-base/95 via-mujin-base/70 to-mujin-base/30"
      />
      <div className="grid gap-8 p-6 md:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] md:items-center md:p-9 lg:p-10">
        <div className="min-w-0">
          <p className="mj-eyebrow text-mujin-copper!">{day}</p>
          <h2 className="mj-display mt-3 text-[44px] text-mujin-text md:text-[56px]">
            Daily Picks
          </h2>
          <p className="mt-3 max-w-md text-[14px] leading-6 text-mujin-text-secondary">
            {lead.reason}. Recommendations from {artists}.
          </p>
          <button
            className="mj-primary mt-6 inline-flex h-10 items-center gap-2 rounded-full px-5 text-[13px] font-semibold"
            onClick={() => void playAlbum(lead.album.id)}
            type="button"
          >
            <PlayIcon className="h-4 w-4" fill="currentColor" />
            Play {lead.album.name}
          </button>
        </div>
        <ol className="grid grid-cols-3 items-end gap-3 md:gap-4">
          {picks.slice(0, 3).map((pick, index) => (
            <li
              className={cn("min-w-0", index === 1 && "md:-translate-y-5")}
              key={pick.album.id}
            >
              <Link
                className="group block"
                href={`/album?id=${encodeURIComponent(pick.album.id)}`}
              >
                <span className="mj-artwork relative block aspect-square overflow-hidden">
                  <Image
                    alt={`${pick.album.name} cover`}
                    className="object-cover transition-transform duration-500 ease-out motion-safe:group-hover:scale-[1.04]"
                    fill
                    sizes="(min-width: 1024px) 220px, 30vw"
                    src={coverUrl(
                      pick.album.cover_url,
                      pick.album.name,
                      pick.album.artist.name,
                    )}
                  />
                </span>
                <span className="mt-2 block truncate text-[12.5px] font-medium text-mujin-text">
                  {pick.album.name}
                </span>
                <span className="block truncate text-[11.5px] text-mujin-text-tertiary">
                  {pick.album.artist.name}
                </span>
              </Link>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

function formatPickDate(date: string) {
  const parsed = new Date(`${date}T12:00:00`);
  if (Number.isNaN(parsed.getTime())) return "Today";
  return parsed.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

/** "In their prime": an artist's best years on a full-bleed band. */
export function BestYearsBand({ prime }: { prime: HomeBestYears }) {
  const playAlbum = usePlayAlbum();
  const years =
    prime.from_year === prime.to_year
      ? String(prime.from_year)
      : `${prime.from_year}–${prime.to_year}`;
  const backdrop = coverUrl(prime.image_url, prime.artist.name);
  return (
    <section
      aria-label={`${prime.artist.name} in ${years}`}
      className="relative isolate overflow-hidden rounded-xl border border-white/[0.07]"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 -z-10 scale-110 bg-cover bg-center opacity-40 blur-2xl saturate-[0.6]"
        style={{ backgroundImage: `url("${backdrop}")` }}
      />
      <div
        aria-hidden="true"
        className="absolute inset-0 -z-10 bg-[radial-gradient(120%_120%_at_0%_100%,rgba(217,139,95,0.22),transparent_55%),linear-gradient(90deg,rgba(10,10,11,0.94),rgba(10,10,11,0.6))]"
      />
      <div className="grid gap-8 p-6 md:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)] md:items-center md:p-9">
        <div>
          <p className="mj-eyebrow">In their prime</p>
          <Link
            className="mj-display mt-3 block text-[40px] text-mujin-text hover:underline md:text-[52px]"
            href={`/artist?id=${encodeURIComponent(prime.artist.id)}`}
          >
            {prime.artist.name}
          </Link>
          <p className="mj-display mt-1 text-[40px] text-mujin-copper md:text-[52px]">
            {years}
          </p>
          <p className="mt-3 text-[13.5px] leading-6 text-mujin-text-secondary">
            {prime.albums.length}{" "}
            {prime.albums.length === 1 ? "release" : "releases"} from their most
            prolific stretch in your library.
          </p>
          {prime.albums[0] && (
            <button
              className="mt-6 inline-flex h-10 items-center gap-2 rounded-full border border-white/15 px-5 text-[13px] font-semibold text-mujin-text transition-colors hover:border-white/30 hover:bg-white/[0.05]"
              onClick={() => void playAlbum(prime.albums[0]!.id)}
              type="button"
            >
              <PlayIcon className="h-4 w-4" fill="currentColor" />
              Play tracks
            </button>
          )}
        </div>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3">
          {prime.albums.slice(0, 3).map((album) => (
            <HomeAlbumCard album={album} key={album.id} />
          ))}
        </div>
      </div>
    </section>
  );
}

/** Genre tiles: a duotone cover collage with the name set large. */
export function GenreTile({ genre }: { genre: HomeGenre }) {
  const covers = genre.covers.length
    ? genre.covers
    : [generatedCover(genre.name)];
  return (
    <Link
      className="group relative block aspect-[4/3] min-w-0 snap-start overflow-hidden rounded-lg border border-white/[0.06] bg-mujin-surface-1"
      href={`/genre?slug=${encodeURIComponent(genre.slug)}`}
    >
      <span
        aria-hidden="true"
        className={cn(
          "absolute inset-0 grid",
          covers.length >= 4 ? "grid-cols-2 grid-rows-2" : "grid-cols-1",
        )}
      >
        {(covers.length >= 4 ? covers.slice(0, 4) : covers.slice(0, 1)).map(
          (cover) => (
            <span className="relative" key={cover}>
              <Image
                alt=""
                className="object-cover grayscale contrast-[1.15] transition-transform duration-500 ease-out motion-safe:group-hover:scale-[1.04]"
                fill
                sizes="240px"
                src={coverUrl(cover, genre.name)}
              />
            </span>
          ),
        )}
      </span>
      {/* Copper duotone: colour blend over the greyscale collage. */}
      <span
        aria-hidden="true"
        className="absolute inset-0 bg-[#b8703f] opacity-80 mix-blend-color transition-opacity duration-300 group-hover:opacity-60"
      />
      <span
        aria-hidden="true"
        className="absolute inset-0 bg-[linear-gradient(to_top,rgba(10,10,11,0.92)_0%,rgba(10,10,11,0.45)_55%,rgba(10,10,11,0.15)_100%)]"
      />
      <span className="absolute inset-x-4 bottom-3.5">
        <span className="mj-title block truncate text-[22px] leading-tight text-mujin-text">
          {genre.name}
        </span>
        <span className="mj-tnum block text-[11.5px] text-mujin-text-secondary">
          {genre.album_count} {genre.album_count === 1 ? "album" : "albums"} ·{" "}
          {genre.artist_count} {genre.artist_count === 1 ? "artist" : "artists"}
        </span>
      </span>
    </Link>
  );
}

/** A saved or recent live station as a play tile. */
export function StationTile({ station }: { station: LiveStation }) {
  const { play, isPlaying } = usePlayStation();
  const playing = isPlaying(station);
  return (
    <div className="mj-card group min-w-0 snap-start">
      <div className="relative">
        <StationArtwork
          className="mj-artwork w-full"
          rounded="rounded-md"
          sizes="220px"
          station={station}
        />
        <button
          aria-label={
            playing ? `Pause ${station.name}` : `Play ${station.name}`
          }
          className="mj-card-play mj-primary absolute bottom-2 right-2 grid h-10 w-10 place-items-center rounded-full"
          onClick={() => play(station)}
          type="button"
        >
          <PlayIcon className="ml-0.5 h-4 w-4" fill="currentColor" />
        </button>
      </div>
      <Link
        className="mt-2.5 block truncate text-[13.5px] font-medium text-mujin-text hover:underline"
        href={stationHref(station.id)}
      >
        {station.name}
      </Link>
      <p className="flex items-center gap-1.5 truncate text-[12px] text-mujin-text-tertiary">
        <Radio aria-hidden="true" className="h-3 w-3 shrink-0" />
        {[
          station.codec?.toUpperCase(),
          station.bitrate_kbps ? `${station.bitrate_kbps} kbps` : null,
          station.country,
        ]
          .filter(Boolean)
          .join(" · ") || "Live radio"}
      </p>
    </div>
  );
}
