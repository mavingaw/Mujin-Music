"use client";

import {
  LIVE_QUALITY_DESCRIPTIONS,
  liveStationSubtitle,
  type LiveCategory,
  type LiveQuality,
  type LiveStation,
} from "@parson/music-sdk";
import { BadgeCheck, Heart, Pause, Play, Radio, Waves } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useState, type ReactNode } from "react";
import { cn } from "@/lib/cn";
import getBaseURL from "@/lib/api/server-url";
import {
  QUALITY_BADGE_CLASSES,
  liveLogoSrc,
  stationHref,
  stationMonogram,
} from "./live-radio-model";
import { usePlayStation, useSaveLiveStation } from "./use-live-radio";

/** Three bars that move while a station is on air here. */
export function OnAirBars({
  className,
  paused = false,
}: {
  className?: string;
  paused?: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn("live-bars inline-flex h-3 items-end gap-[2px]", className)}
      data-paused={paused || undefined}
    >
      <span />
      <span />
      <span />
    </span>
  );
}

/** A small red "LIVE" pill. */
export function LiveBadge({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full bg-red-500/15 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-[0.12em] text-red-300 ring-1 ring-inset ring-red-400/30",
        className,
      )}
    >
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-red-400" />
      Live
    </span>
  );
}

export function QualityBadge({
  quality,
  className,
}: {
  quality: LiveQuality;
  className?: string;
}) {
  if (quality.tier === "unknown" && quality.label === "Unknown format") {
    return null;
  }
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-[5px] border px-1.5 py-[1px] text-[10px] font-semibold uppercase tracking-wide tabular-nums",
        QUALITY_BADGE_CLASSES[quality.tier],
        className,
      )}
      title={`${LIVE_QUALITY_DESCRIPTIONS[quality.tier]} ${
        quality.verified
          ? "Parson read this from the station's audio."
          : "As the station is listed; Parson checks when it plays."
      }`}
    >
      {quality.verified && (
        <BadgeCheck aria-label="Verified" className="-ml-0.5 mr-1 h-3 w-3" />
      )}
      {quality.label}
    </span>
  );
}

/**
 * A station's logo on a tile. Logos come in every shape and size, so the
 * logo sits contained over a blurred copy of itself; stations without one
 * get a monogram on a gradient derived from their name.
 */
export function StationArtwork({
  station,
  className,
  sizes = "200px",
  rounded = "rounded-xl",
  children,
}: {
  station: Pick<LiveStation, "name" | "logo_url">;
  className?: string;
  sizes?: string;
  rounded?: string;
  children?: ReactNode;
}) {
  const [failed, setFailed] = useState(false);
  const logo = failed ? null : liveLogoSrc(station.logo_url, getBaseURL());
  const monogram = stationMonogram(station.name);
  return (
    <div
      className={cn(
        "relative aspect-square overflow-hidden bg-zinc-900 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.07)]",
        rounded,
        className,
      )}
      style={
        logo
          ? undefined
          : {
              backgroundImage: `radial-gradient(circle at 25% 20%, rgba(255,255,255,0.18), transparent 55%), linear-gradient(140deg, ${monogram.from}, ${monogram.to})`,
            }
      }
    >
      {logo ? (
        <>
          <Image
            alt=""
            aria-hidden="true"
            className="scale-125 object-cover opacity-45 blur-2xl saturate-150"
            fill
            sizes="64px"
            src={logo}
            unoptimized
          />
          <div className="absolute inset-[14%] overflow-hidden rounded-lg">
            <Image
              alt={station.name}
              className="object-contain drop-shadow-[0_6px_18px_rgba(0,0,0,0.45)]"
              fill
              onError={() => setFailed(true)}
              sizes={sizes}
              src={logo}
              unoptimized
            />
          </div>
        </>
      ) : (
        <span className="absolute inset-0 grid place-items-center text-[clamp(1.25rem,28cqw,3.5rem)] font-black tracking-tight text-white/90 [container-type:inline-size]">
          {monogram.initials}
        </span>
      )}
      {children}
    </div>
  );
}

export function SaveStationButton({
  station,
  className,
  size = "sm",
}: {
  station: LiveStation;
  className?: string;
  size?: "sm" | "lg";
}) {
  const save = useSaveLiveStation();
  const label = station.saved
    ? `Remove ${station.name} from My Live Radio`
    : `Add ${station.name} to My Live Radio`;
  return (
    <button
      aria-label={label}
      aria-pressed={station.saved}
      className={cn(
        "grid shrink-0 place-items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/30",
        size === "lg"
          ? "h-11 w-11 border border-white/15 hover:border-white/40"
          : "h-8 w-8 hover:bg-white/10",
        station.saved ? "text-rose-400" : "text-zinc-400 hover:text-white",
        className,
      )}
      disabled={save.isPending}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        save.mutate({ station, saved: station.saved });
      }}
      title={label}
      type="button"
    >
      <Heart
        className={size === "lg" ? "h-5 w-5" : "h-4 w-4"}
        fill={station.saved ? "currentColor" : "none"}
      />
    </button>
  );
}

function PlayOverlay({ station }: { station: LiveStation }) {
  const player = usePlayStation();
  const playing = player.isPlaying(station);
  const current = player.isCurrent(station);
  return (
    <>
      {current && (
        <span className="absolute left-2 top-2 inline-flex items-center gap-1.5 rounded-full bg-black/70 px-2 py-1 text-[10px] font-bold uppercase tracking-wider text-white backdrop-blur">
          <OnAirBars paused={!playing} />
          {playing ? "On air" : "Paused"}
        </span>
      )}
      <button
        aria-label={playing ? `Pause ${station.name}` : `Play ${station.name}`}
        className={cn(
          "absolute bottom-2.5 right-2.5 grid h-11 w-11 place-items-center rounded-full bg-white text-black shadow-xl shadow-black/50 transition-all duration-200 hover:scale-105 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/60",
          current
            ? "translate-y-0 opacity-100"
            : "translate-y-2 opacity-0 group-hover:translate-y-0 group-hover:opacity-100",
        )}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          player.play(station);
        }}
        type="button"
      >
        {playing ? (
          <Pause className="h-5 w-5" fill="currentColor" />
        ) : (
          <Play className="ml-0.5 h-5 w-5" fill="currentColor" />
        )}
      </button>
    </>
  );
}

/** A station in a grid: logo, name, place and genres, quality. */
export function StationCard({
  station,
  subtitle,
  asItem = true,
}: {
  station: LiveStation;
  subtitle?: string;
  /** Renders an `<li>` (default) or a `<div>` inside a caller's item. */
  asItem?: boolean;
}) {
  const Item = asItem ? "li" : "div";
  return (
    <Item className="group min-w-0 list-none">
      <Link
        className="block focus-visible:outline-none"
        href={stationHref(station.id)}
      >
        <StationArtwork
          className="transition-transform duration-300 group-hover:-translate-y-0.5 group-focus-visible:ring-2 group-focus-visible:ring-white/40"
          station={station}
          sizes="(min-width: 1280px) 200px, (min-width: 768px) 22vw, 45vw"
        >
          <PlayOverlay station={station} />
        </StationArtwork>
      </Link>
      <div className="mt-2.5 flex min-w-0 items-start gap-1">
        <div className="min-w-0 flex-1">
          <Link
            className="block truncate text-sm font-semibold text-white hover:underline"
            href={stationHref(station.id)}
            title={station.name}
          >
            {station.name}
          </Link>
          <p className="truncate text-xs text-zinc-400">
            {subtitle ?? liveStationSubtitle(station)}
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-1">
            <QualityBadge quality={station.quality} />
            {station.hls && (
              <span className="rounded-[5px] border border-white/10 px-1.5 py-[1px] text-[10px] font-semibold uppercase tracking-wide text-zinc-400">
                HLS
              </span>
            )}
            {station.online === false && (
              <span className="text-[10px] font-medium text-amber-300/80">
                Offline lately
              </span>
            )}
          </div>
        </div>
        <SaveStationButton station={station} />
      </div>
    </Item>
  );
}

export function StationGrid({
  stations,
  label,
}: {
  stations: LiveStation[];
  label: string;
}) {
  return (
    <ul
      aria-label={label}
      className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 2xl:grid-cols-6"
    >
      {stations.map((station) => (
        <StationCard key={station.id} station={station} />
      ))}
    </ul>
  );
}

/** One horizontal row of station cards that scrolls sideways. */
export function StationShelf({
  stations,
  label,
}: {
  stations: LiveStation[];
  label: string;
}) {
  return (
    <ul
      aria-label={label}
      className="app-shell-scroll -mx-1 flex snap-x gap-4 overflow-x-auto px-1 pb-2 [&>li]:w-[42vw] [&>li]:shrink-0 [&>li]:snap-start sm:[&>li]:w-44 lg:[&>li]:w-48"
    >
      {stations.map((station) => (
        <StationCard key={station.id} station={station} />
      ))}
    </ul>
  );
}

/** A compact station row (lists, search results on phones). */
export function StationRow({
  station,
  trailing,
}: {
  station: LiveStation;
  trailing?: ReactNode;
}) {
  const player = usePlayStation();
  const current = player.isCurrent(station);
  return (
    <li className="group flex min-w-0 items-center gap-3 rounded-lg px-2 py-2 transition-colors hover:bg-white/5">
      <button
        aria-label={`Play ${station.name}`}
        className="relative shrink-0"
        onClick={() => player.play(station)}
        type="button"
      >
        <StationArtwork
          className="h-12 w-12"
          rounded="rounded-md"
          sizes="48px"
          station={station}
        />
        <span className="absolute inset-0 grid place-items-center rounded-md bg-black/50 opacity-0 transition-opacity group-hover:opacity-100">
          <Play className="h-4 w-4 text-white" fill="currentColor" />
        </span>
      </button>
      <div className="min-w-0 flex-1">
        <Link
          className={cn(
            "flex items-center gap-2 truncate text-sm font-semibold hover:underline",
            current ? "text-emerald-300" : "text-white",
          )}
          href={stationHref(station.id)}
        >
          {current && <OnAirBars paused={!player.isPlaying(station)} />}
          <span className="truncate">{station.name}</span>
        </Link>
        <p className="truncate text-xs text-zinc-400">
          {liveStationSubtitle(station)}
        </p>
      </div>
      <QualityBadge
        className="hidden sm:inline-flex"
        quality={station.quality}
      />
      {trailing ?? <SaveStationButton station={station} />}
    </li>
  );
}

const CATEGORY_ICONS: Record<string, ReactNode> = {
  lossless: <Waves className="h-16 w-16" strokeWidth={1.2} />,
};

/** A genre or collection tile, Roon-style, on a generated gradient. */
export function CategoryTile({
  category,
  href,
  large = false,
}: {
  category: LiveCategory;
  href: string;
  large?: boolean;
}) {
  const [from, to] = category.colors;
  return (
    <Link
      className={cn(
        "group relative isolate flex overflow-hidden rounded-xl p-4 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.08)] transition-transform duration-300 hover:-translate-y-0.5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/40",
        large ? "h-36 items-end md:h-44" : "h-24 items-end md:h-28",
      )}
      href={href}
      style={{
        backgroundImage: `radial-gradient(120% 90% at 100% 0%, rgba(255,255,255,0.22), transparent 55%), radial-gradient(80% 80% at 0% 100%, rgba(0,0,0,0.55), transparent 70%), linear-gradient(135deg, ${from}, ${to})`,
      }}
    >
      <span className="pointer-events-none absolute -right-3 -top-3 text-white/15 transition-transform duration-500 group-hover:rotate-6 group-hover:scale-110">
        {CATEGORY_ICONS[category.id] ?? (
          <Radio className="h-16 w-16" strokeWidth={1.2} />
        )}
      </span>
      <span
        className={cn(
          "relative font-bold leading-tight text-white drop-shadow-[0_2px_10px_rgba(0,0,0,0.5)]",
          large ? "text-2xl md:text-3xl" : "text-base md:text-lg",
        )}
      >
        {category.label}
      </span>
    </Link>
  );
}
