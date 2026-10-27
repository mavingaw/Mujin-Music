"use client";

import { getPopularGenres, radioStationLabel } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Radio, ThumbsDown, ThumbsUp } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { usePlayer } from "../player-api";

type ThumbSize = "sm" | "md" | "lg";

const thumbButton: Record<ThumbSize, string> = {
  sm: "h-7 w-7",
  md: "h-9 w-9",
  lg: "h-11 w-11",
};
const thumbIcon: Record<ThumbSize, string> = {
  sm: "h-3.5 w-3.5",
  md: "h-4 w-4",
  lg: "h-5 w-5",
};

/** Thumbs up / down for a queue item (the current track or a Radio pick). */
export function RadioThumbs({
  className = "",
  index,
  label,
  liked = false,
  size = "md",
  solid = false,
}: {
  className?: string;
  index: number;
  label: string;
  liked?: boolean;
  size?: ThumbSize;
  /** Roon-style filled circles, used by the coming-up notice. */
  solid?: boolean;
}) {
  const { radio, currentIndex } = usePlayer();
  const current = index === currentIndex;
  const up = solid
    ? `${liked ? "bg-mujin-copper-hover text-mujin-on-copper" : "bg-mujin-copper text-mujin-on-copper hover:bg-mujin-copper-hover"}`
    : `${liked ? "text-mujin-copper" : "text-mujin-text-secondary hover:bg-white/10 hover:text-mujin-text"}`;
  const down = solid
    ? "bg-white text-black hover:bg-zinc-200"
    : "text-zinc-400 hover:bg-white/10 hover:text-white";
  return (
    <span className={`flex items-center gap-1 ${className}`}>
      <button
        aria-label={
          liked ? `You liked ${label}` : `Thumbs up: more like ${label}`
        }
        aria-pressed={liked}
        className={`grid shrink-0 place-items-center rounded-full transition-colors ${thumbButton[size]} ${up}`}
        onClick={(event) => {
          event.stopPropagation();
          radio.thumbsUp(index);
        }}
        title="More like this"
        type="button"
      >
        <ThumbsUp
          className={`${thumbIcon[size]} ${liked ? "fill-current" : ""}`}
        />
      </button>
      <button
        aria-label={
          current
            ? `Thumbs down: skip ${label} and play less like it`
            : `Thumbs down: replace ${label}`
        }
        className={`grid shrink-0 place-items-center rounded-full transition-colors ${thumbButton[size]} ${down}`}
        onClick={(event) => {
          event.stopPropagation();
          radio.thumbsDown(index);
        }}
        title={current ? "Skip and play less like this" : "Replace this pick"}
        type="button"
      >
        <ThumbsDown className={thumbIcon[size]} />
      </button>
    </span>
  );
}

/** An accessible on/off switch. */
export function RadioSwitch({
  checked,
  label,
  onChange,
}: {
  checked: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={`relative inline-flex h-6 w-10.5 shrink-0 items-center rounded-full border transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white ${
        checked
          ? "border-mujin-copper bg-mujin-copper"
          : "border-white/15 bg-white/10 hover:bg-white/15"
      }`}
      onClick={(event) => {
        event.stopPropagation();
        onChange(!checked);
      }}
      role="switch"
      type="button"
    >
      <span
        aria-hidden="true"
        className={`h-4.5 w-4.5 rounded-full bg-mujin-text shadow transition-transform ${
          checked ? "translate-x-5" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}

/** Header of the Radio section at the end of the queue. */
export function RadioQueueSection({
  hasPicks,
  showStations = true,
}: {
  hasPicks: boolean;
  showStations?: boolean;
}) {
  const { radio } = usePlayer();
  return (
    <section
      aria-label="Radio"
      className="mx-1 mb-1 mt-3 border-t border-white/10 px-1 pt-3"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="flex items-center gap-2 text-[15px] font-semibold text-white">
            <Radio
              aria-hidden="true"
              className={`h-4 w-4 ${radio.enabled ? "text-mujin-copper" : "text-mujin-text-disabled"}`}
            />
            Radio
          </p>
          <p className="mt-0.5 truncate text-xs leading-5 text-zinc-500">
            {radio.enabled
              ? radioStationLabel(radio.seed)
              : "Off. Playback stops when your queue ends."}
          </p>
        </div>
        <span className="flex items-center gap-2 pt-0.5 text-xs text-zinc-400">
          <span aria-hidden="true">{radio.enabled ? "On" : "Off"}</span>
          <RadioSwitch
            checked={radio.enabled}
            label="Radio: keep playing similar music when the queue ends"
            onChange={radio.setEnabled}
          />
        </span>
      </div>
      {radio.enabled && !hasPicks && (
        <p className="mt-2 text-xs leading-5 text-zinc-500">
          Picking music based on your queue…
        </p>
      )}
      {radio.enabled && showStations && <GenreStations />}
    </section>
  );
}

/** Quick genre stations, so Radio can start from a genre. */
export function GenreStations({ limit = 8 }: { limit?: number }) {
  const { radio } = usePlayer();
  const [starting, setStarting] = useState<string | null>(null);
  const genres = useQuery({
    queryKey: ["genres", "popular"],
    queryFn: getPopularGenres,
    staleTime: 30 * 60_000,
  });
  const stations = (genres.data ?? []).slice(0, limit);
  if (!stations.length) {
    return null;
  }
  return (
    <div className="mt-2">
      <p className="sr-only">Start a genre station</p>
      <div className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-1 [scrollbar-width:none]">
        {stations.map((genre) => {
          const active =
            radio.seed?.kind === "genre" &&
            radio.seed.id.toLowerCase() === genre.name.toLowerCase();
          return (
            <button
              aria-label={`Start ${genre.name} radio`}
              aria-pressed={active}
              className={`shrink-0 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors disabled:opacity-60 ${
                active
                  ? "border-mujin-copper/60 bg-mujin-copper/15 text-mujin-copper-hover"
                  : "border-white/10 text-zinc-300 hover:border-white/25 hover:text-white"
              }`}
              disabled={starting !== null}
              key={genre.name}
              onClick={(event) => {
                event.stopPropagation();
                setStarting(genre.name);
                void radio
                  .start({ kind: "genre", id: genre.name })
                  .then((started) => {
                    if (started === null)
                      toast("Nothing to play for this genre yet.");
                  })
                  .catch(() => toast("Could not start this station."))
                  .finally(() => setStarting(null));
              }}
              type="button"
            >
              {genre.name}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** Starts a station and reports the outcome; shared by every "Start radio". */
export function useStartRadio() {
  const { radio } = usePlayer();
  return (seed: Parameters<typeof radio.start>[0]) =>
    radio
      .start(seed)
      .then((station) => {
        if (station === null)
          toast("Radio has nothing to add yet. Play a few songs first.");
        else toast.success(station ? `Playing ${station} radio` : "Radio on");
      })
      .catch(() => toast("Could not start radio."));
}

/** Pill button that starts a station from an album or artist page. */
export function StartRadioButton({
  label,
  seed,
}: {
  label: string;
  seed: Parameters<ReturnType<typeof usePlayer>["radio"]["start"]>[0];
}) {
  const startRadio = useStartRadio();
  const [starting, setStarting] = useState(false);
  return (
    <button
      aria-label={label}
      className="inline-flex h-10 items-center gap-2 rounded-full border border-white/15 px-4 text-sm font-semibold text-zinc-100 transition-colors hover:border-white/35 hover:bg-white/6 disabled:opacity-60"
      disabled={starting}
      onClick={() => {
        setStarting(true);
        void startRadio(seed).finally(() => setStarting(false));
      }}
      title={label}
      type="button"
    >
      <Radio aria-hidden="true" className="h-4 w-4 text-mujin-copper" />
      Radio
    </button>
  );
}
