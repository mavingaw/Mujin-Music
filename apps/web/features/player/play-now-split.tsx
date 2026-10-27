"use client";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ChevronDown,
  ListEnd,
  ListStart,
  Pause,
  Play,
  Radio,
} from "lucide-react";
import { useState, type ReactNode } from "react";
import {
  PLAY_ACTIONS,
  usePlayActions,
  type PlayAction,
  type PlayTarget,
} from "./play-actions";

const ACTION_ICONS: Record<PlayAction, ReactNode> = {
  play: <Play className="h-4 w-4 fill-current" />,
  next: <ListStart className="h-4 w-4" />,
  queue: <ListEnd className="h-4 w-4" />,
  radio: <Radio className="h-4 w-4" />,
};

/**
 * Roon's split "Play now ▾": the main half plays, the chevron offers Play
 * now, Add next, Add to queue and Start radio. `hero` is the page-header
 * pill, `row` a compact pair for list rows (shown on hover and focus),
 * `card` two round buttons over artwork.
 */
export function PlayNowSplit({
  target,
  variant = "hero",
  playing = false,
  busy = false,
  onPlayNow,
  queueLength,
  children,
  className = "",
}: {
  target: PlayTarget;
  variant?: "hero" | "row" | "card";
  /** The target is playing: the main half pauses instead. */
  playing?: boolean;
  busy?: boolean;
  /** Replaces the main half's Play now (album pages pick a preferred copy,
   * the playing item toggles). */
  onPlayNow?: () => void;
  /** Tracks in the queue, shown next to Add to queue. */
  queueLength?: number;
  /** More items under the play choices ("View lyrics", ...). */
  children?: ReactNode;
  className?: string;
}) {
  const run = usePlayActions();
  const [open, setOpen] = useState(false);
  const name = target.name ?? `this ${target.kind}`;
  const choose = (action: PlayAction) => {
    if (action === "play" && onPlayNow) onPlayNow();
    else void run(target, action);
  };
  const menu = (
    <DropdownMenuContent
      align={variant === "hero" ? "start" : "end"}
      className="w-56"
      onClick={(event) => event.stopPropagation()}
    >
      <DropdownMenuLabel className="truncate text-[11px] font-semibold uppercase tracking-[0.12em] text-mujin-text-tertiary">
        {name}
      </DropdownMenuLabel>
      {PLAY_ACTIONS.map(({ action, label }) => (
        <DropdownMenuItem key={action} onSelect={() => choose(action)}>
          {ACTION_ICONS[action]}
          <span className="flex-1">{label}</span>
          {action === "queue" && queueLength ? (
            <span className="mj-tnum text-[11px] text-mujin-text-tertiary">
              {queueLength} queued
            </span>
          ) : null}
        </DropdownMenuItem>
      ))}
      {children && <DropdownMenuSeparator />}
      {children}
    </DropdownMenuContent>
  );
  const mainLabel = `${playing ? "Pause" : "Play"} ${name}`;
  const moreLabel = `More ways to play ${name}`;

  if (variant === "hero") {
    return (
      <DropdownMenu onOpenChange={setOpen} open={open}>
        <div
          className={`inline-flex h-11 items-stretch overflow-hidden rounded-full shadow-[0_10px_30px_-10px_rgba(217,139,95,0.6)] ${className}`}
          data-split="play-now"
        >
          <button
            aria-label={mainLabel}
            className="mj-primary inline-flex items-center gap-2 pl-4 pr-4 text-[13.5px] font-semibold disabled:opacity-70"
            disabled={busy}
            onClick={() => choose("play")}
            type="button"
          >
            {playing ? (
              <Pause className="h-4.5 w-4.5 fill-current" />
            ) : (
              <Play className="h-4.5 w-4.5 fill-current" />
            )}
            {playing ? "Pause" : "Play now"}
          </button>
          <span aria-hidden="true" className="w-px bg-mujin-on-copper/25" />
          <DropdownMenuTrigger asChild>
            <button
              aria-label={moreLabel}
              className="mj-primary inline-flex w-10 items-center justify-center data-[state=open]:bg-mujin-copper-pressed"
              type="button"
            >
              <ChevronDown className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
        </div>
        {menu}
      </DropdownMenu>
    );
  }

  if (variant === "card") {
    return (
      <DropdownMenu onOpenChange={setOpen} open={open}>
        <div
          className={`pointer-events-auto flex items-center gap-1.5 ${className}`}
          data-split="play-now"
        >
          <DropdownMenuTrigger asChild>
            <button
              aria-label={moreLabel}
              className="grid h-8 w-8 place-items-center rounded-full border border-white/15 bg-black/55 text-mujin-text backdrop-blur-md transition-colors hover:bg-black/75 data-[state=open]:bg-black/80"
              onClick={(event) => event.stopPropagation()}
              type="button"
            >
              <ChevronDown className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <button
            aria-label={mainLabel}
            className="mj-primary grid h-10 w-10 place-items-center rounded-full shadow-[0_8px_24px_-6px_rgba(0,0,0,0.8)]"
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              choose("play");
            }}
            type="button"
          >
            {playing ? (
              <Pause className="h-4 w-4 fill-current" />
            ) : (
              <Play className="ml-0.5 h-4 w-4 fill-current" />
            )}
          </button>
        </div>
        {menu}
      </DropdownMenu>
    );
  }

  return (
    <DropdownMenu onOpenChange={setOpen} open={open}>
      <div
        className={`relative z-20 inline-flex h-8 items-stretch overflow-hidden rounded-full border border-white/12 bg-mujin-surface-2/80 text-mujin-text-secondary transition-opacity focus-within:opacity-100 group-hover:opacity-100 pointer-coarse:opacity-100 ${open ? "opacity-100" : "opacity-0"} ${className}`}
        data-split="play-now"
      >
        <button
          aria-label={mainLabel}
          className="inline-flex w-8 items-center justify-center transition-colors hover:bg-mujin-copper hover:text-mujin-on-copper"
          onClick={(event) => {
            event.stopPropagation();
            choose("play");
          }}
          type="button"
        >
          {playing ? (
            <Pause className="h-3.5 w-3.5 fill-current" />
          ) : (
            <Play className="ml-px h-3.5 w-3.5 fill-current" />
          )}
        </button>
        <span aria-hidden="true" className="w-px bg-white/10" />
        <DropdownMenuTrigger asChild>
          <button
            aria-label={moreLabel}
            className="inline-flex w-7 items-center justify-center transition-colors hover:bg-white/10 hover:text-mujin-text data-[state=open]:bg-white/10 data-[state=open]:text-mujin-text"
            onClick={(event) => event.stopPropagation()}
            type="button"
          >
            <ChevronDown className="h-3.5 w-3.5" />
          </button>
        </DropdownMenuTrigger>
      </div>
      {menu}
    </DropdownMenu>
  );
}
