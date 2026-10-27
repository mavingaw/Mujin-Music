"use client";

import {
  LIVE_QUALITY_DESCRIPTIONS,
  liveCodecLabel,
  liveCountryName,
  liveFormatText,
  liveTitleText,
  type LiveNowPlaying,
  type LiveStation,
} from "@parson/music-sdk";
import {
  ArrowLeft,
  Copy,
  ExternalLink,
  Globe2,
  Heart,
  Languages,
  Loader2,
  MapPin,
  Pause,
  Play,
  ShieldCheck,
  Signal,
} from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { usePageTitle } from "@/components/app/title-metadata";
import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import {
  compactCount,
  liveLogoSrc,
  paramsFromSearch,
} from "./live-radio-model";
import {
  LiveBadge,
  OnAirBars,
  QualityBadge,
  StationArtwork,
  StationShelf,
} from "./station-ui";
import {
  liveRadioError,
  useLiveNowPlaying,
  useLiveStation,
  usePlayStation,
  useSaveLiveStation,
  useVerifyLiveStation,
} from "./use-live-radio";

function hostOf(url: string) {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

function Fact({
  icon,
  label,
  children,
}: {
  icon: ReactNode;
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 gap-3 rounded-xl border border-white/8 bg-white/[0.025] p-4">
      <span className="mt-0.5 text-zinc-500">{icon}</span>
      <div className="min-w-0">
        <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
          {label}
        </p>
        <div className="mt-1 text-sm text-zinc-100">{children}</div>
      </div>
    </div>
  );
}

function OnAir({
  station,
  playing,
  here,
}: {
  station: LiveStation;
  playing: LiveNowPlaying | null | undefined;
  here: boolean;
}) {
  const title = playing?.title;
  const history = playing?.history ?? [];
  if (!title && !history.length) {
    return (
      <p className="text-sm leading-6 text-zinc-500">
        {here
          ? `${station.name} is not sending track titles right now.`
          : "Play the station to see what is on air. Titles come from the stream itself."}
      </p>
    );
  }
  const time = (at: number) =>
    new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return (
    <div>
      {title && (
        <div className="flex items-center gap-3 rounded-xl bg-gradient-to-r from-rose-500/15 to-transparent p-4 ring-1 ring-inset ring-rose-400/15">
          <OnAirBars className="h-4 text-rose-300" paused={!playing?.live} />
          <div className="min-w-0">
            <p className="truncate text-base font-semibold text-white">
              {title.title}
            </p>
            {title.artist && (
              <p className="truncate text-sm text-zinc-300">{title.artist}</p>
            )}
          </div>
          <span className="ml-auto shrink-0 text-xs tabular-nums text-zinc-500">
            since {time(title.at)}
          </span>
        </div>
      )}
      {history.length > 0 && (
        <ol className="mt-3 divide-y divide-white/5">
          {history.map((item) => (
            <li
              className="flex items-center gap-3 py-2 text-sm"
              key={`${item.at}-${item.raw}`}
            >
              <span className="w-12 shrink-0 tabular-nums text-xs text-zinc-500">
                {time(item.at)}
              </span>
              <span className="truncate text-zinc-300">
                {liveTitleText(item)}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

function SignalSummary({
  station,
  playing,
  checking,
  failed,
}: {
  station: LiveStation;
  playing: LiveNowPlaying | null | undefined;
  checking: boolean;
  failed: boolean;
}) {
  const format = playing?.format ?? station.verified_format ?? null;
  const quality = station.quality.verified
    ? station.quality
    : (playing?.quality ?? station.quality);
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <QualityBadge quality={quality} />
        <span className="text-sm text-zinc-200">
          {format
            ? liveFormatText(format)
            : [
                liveCodecLabel(station.codec),
                station.bitrate_kbps && !quality.lossless
                  ? `${station.bitrate_kbps} kbps`
                  : null,
              ]
                .filter(Boolean)
                .join(" · ")}
        </span>
      </div>
      <p className="text-xs leading-5 text-zinc-500">
        {checking ? (
          <span className="inline-flex items-center gap-1.5 text-zinc-400">
            <Loader2 className="h-3 w-3 animate-spin" /> Listening to the
            station to check its real format…
          </span>
        ) : format?.verified ? (
          "Read from the stream's audio frames by your Mujin Music server."
        ) : failed ? (
          "The station did not answer Mujin Music's check; this is its listing."
        ) : (
          "As the directory lists it; Mujin Music confirms it from the audio once the station plays."
        )}{" "}
        {LIVE_QUALITY_DESCRIPTIONS[quality.tier]}
      </p>
      {playing?.delivery && playing.delivery.mode !== "original" && (
        <p className="text-xs text-zinc-400">
          {playing.delivery.mode === "repackage"
            ? "HLS segments joined into one stream, without re-encoding."
            : `Converted to ${playing.delivery.output.toUpperCase()} for an output that cannot play ${liveCodecLabel(format?.codec)}.`}
        </p>
      )}
    </div>
  );
}

/**
 * A station: logo, place, genres, what it is playing (with the titles it
 * played before), its real format, website and similar stations.
 */
export default function StationDetails() {
  const params = useSearchParams();
  const id = params?.get("id") ?? null;
  const query = useLiveStation(id);
  const player = usePlayStation();
  const save = useSaveLiveStation();
  const station = query.data?.station;
  const current = station ? player.isCurrent(station) : false;
  const playingHere = station ? player.isPlaying(station) : false;
  const live = useLiveNowPlaying(station?.id ?? null, current);
  const verifying = useVerifyLiveStation(station);
  const playing = live.data ?? query.data?.now_playing;
  usePageTitle(station?.name ?? "Station");

  if (query.isPending) {
    return (
      <div className="flex justify-center pt-24 text-zinc-500">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }
  if (query.isError || !station) {
    return (
      <div className="mx-auto max-w-225 px-7 pt-16 text-zinc-400">
        <p>{liveRadioError(query.error, "This station could not be found.")}</p>
        <Link
          className="mt-3 inline-block text-sm text-white hover:underline"
          href="/radio"
        >
          Back to Live Radio
        </Link>
      </div>
    );
  }
  const logo = liveLogoSrc(station.logo_url, getBaseURL());
  const country = liveCountryName(station.country_code, station.country);
  const place = [station.region, country]
    .filter((part, index, parts) => part && parts.indexOf(part) === index)
    .join(", ");
  const listens = compactCount(station.clicks);
  const votes = compactCount(station.votes);

  return (
    <div className="relative isolate min-h-full overflow-x-clip pb-36 text-zinc-50">
      <div
        aria-hidden="true"
        className="absolute inset-x-0 top-0 -z-10 h-[460px] overflow-hidden"
      >
        {logo ? (
          <Image
            alt=""
            className="scale-150 object-cover opacity-30 blur-3xl saturate-150"
            fill
            sizes="100vw"
            src={logo}
            unoptimized
          />
        ) : (
          <div className="h-full bg-[radial-gradient(60%_80%_at_30%_0%,rgba(217,139,95,0.22),transparent_70%)]" />
        )}
        <div className="absolute inset-0 bg-gradient-to-b from-black/30 via-black/70 to-black" />
      </div>
      <div className="mx-auto w-full max-w-310 px-5 pt-6 sm:px-8">
        <Link
          className="inline-flex items-center gap-1.5 text-sm text-zinc-400 hover:text-white"
          href="/radio"
        >
          <ArrowLeft className="h-4 w-4" /> Live Radio
        </Link>
        <header className="mt-8 flex flex-col gap-6 sm:flex-row sm:items-end sm:gap-8">
          <StationArtwork
            className="w-44 shrink-0 shadow-2xl shadow-black/60 sm:w-60"
            rounded="rounded-2xl"
            sizes="240px"
            station={station}
          />
          <div className="min-w-0 flex-1">
            <p className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.16em] text-zinc-400">
              {current && playingHere ? <LiveBadge /> : "Live radio"}
              {station.source === "custom" && (
                <span className="text-zinc-500">· Added by URL</span>
              )}
            </p>
            <h1 className="mt-2 text-balance text-4xl mj-display leading-[1.05] text-mujin-text sm:text-6xl">
              {station.name}
            </h1>
            {place && (
              <p className="mt-3 flex items-center gap-1.5 text-sm text-zinc-300">
                <MapPin className="h-4 w-4 text-zinc-500" /> {place}
              </p>
            )}
            <div className="mt-6 flex flex-wrap items-center gap-3">
              <button
                className="inline-flex h-12 items-center gap-2 rounded-full bg-white px-7 text-sm font-bold text-black shadow-lg shadow-black/40 transition-transform hover:scale-[1.03]"
                onClick={() => player.play(station)}
                type="button"
              >
                {playingHere ? (
                  <Pause className="h-5 w-5" fill="currentColor" />
                ) : (
                  <Play className="h-5 w-5" fill="currentColor" />
                )}
                {playingHere ? "Pause" : current ? "Resume" : "Play now"}
              </button>
              <button
                aria-pressed={station.saved}
                className={cn(
                  "inline-flex h-12 items-center gap-2 rounded-full border px-5 text-sm font-semibold transition-colors",
                  station.saved
                    ? "border-rose-400/40 bg-rose-500/10 text-rose-200"
                    : "border-white/20 text-white hover:border-white/40",
                )}
                disabled={save.isPending}
                onClick={() => save.mutate({ station, saved: station.saved })}
                type="button"
              >
                <Heart
                  className="h-4 w-4"
                  fill={station.saved ? "currentColor" : "none"}
                />
                {station.saved ? "In My Live Radio" : "Add to My Live Radio"}
              </button>
              {player.target.kind === "zone" && (
                <span className="text-xs text-emerald-300">
                  Plays on {player.target.name}
                </span>
              )}
            </div>
          </div>
        </header>

        {station.tags.length > 0 && (
          <ul className="mt-8 flex flex-wrap gap-1.5">
            {station.tags.map((tag) => (
              <li key={tag}>
                <Link
                  className="inline-flex rounded-full border border-white/12 bg-white/5 px-3 py-1 text-xs font-medium uppercase tracking-wide text-zinc-200 transition-colors hover:border-white/30 hover:bg-white/10"
                  href={`/radio?${paramsFromSearch({ tag })}`}
                >
                  {tag}
                </Link>
              </li>
            ))}
          </ul>
        )}

        <div className="mt-10 grid gap-8 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
          <section>
            <h2 className="mb-4 flex items-center gap-2 text-xl font-bold text-white">
              On air
            </h2>
            <OnAir here={current} playing={playing} station={station} />
            {station.description && (
              <p className="mt-6 text-sm leading-6 text-zinc-400">
                {station.description}
              </p>
            )}
          </section>
          <section className="grid content-start gap-3">
            <Fact icon={<Signal className="h-4 w-4" />} label="Stream quality">
              <SignalSummary
                checking={verifying.isFetching}
                failed={verifying.isError}
                playing={playing}
                station={station}
              />
            </Fact>
            <div className="grid gap-3 sm:grid-cols-2">
              {country && (
                <Fact icon={<Globe2 className="h-4 w-4" />} label="Country">
                  <Link
                    className="hover:underline"
                    href={`/radio?${paramsFromSearch({ country_code: station.country_code ?? undefined })}`}
                  >
                    {country}
                  </Link>
                </Fact>
              )}
              {station.languages.length > 0 && (
                <Fact icon={<Languages className="h-4 w-4" />} label="Language">
                  <span className="capitalize">
                    {station.languages.join(", ")}
                  </span>
                </Fact>
              )}
              {(listens || votes) && (
                <Fact icon={<Heart className="h-4 w-4" />} label="Community">
                  {[listens && `${listens} listens`, votes && `${votes} votes`]
                    .filter(Boolean)
                    .join(" · ")}
                </Fact>
              )}
              <Fact icon={<ShieldCheck className="h-4 w-4" />} label="Delivery">
                {station.hls ? "HLS" : "Icecast / HTTP"} through your Mujin
                Music server
              </Fact>
            </div>
            {station.homepage && (
              <a
                className="inline-flex items-center gap-2 text-sm text-zinc-300 hover:text-white"
                href={station.homepage}
                rel="noreferrer"
                target="_blank"
              >
                <ExternalLink className="h-4 w-4" /> {hostOf(station.homepage)}
              </a>
            )}
            <button
              className="inline-flex w-fit items-center gap-2 text-xs text-zinc-500 hover:text-zinc-300"
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(station.stream_url)
                  .then(() => toast.success("Stream URL copied"));
              }}
              title={station.stream_url}
              type="button"
            >
              <Copy className="h-3.5 w-3.5" /> Copy stream URL
            </button>
          </section>
        </div>

        {(query.data?.similar.length ?? 0) > 0 && (
          <section className="mt-14">
            <h2 className="mb-4 border-b border-white/8 pb-3 text-xl font-bold text-white sm:text-2xl">
              More like {station.name}
            </h2>
            <StationShelf
              label="Similar stations"
              stations={query.data!.similar}
            />
          </section>
        )}
      </div>
    </div>
  );
}
