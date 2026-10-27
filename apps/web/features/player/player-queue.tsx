"use client";

import { defaultCover } from "@/lib/images/default-cover";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import { usePlayer, type Player } from "./player-api";
import {
  type ZoneRepeatMode,
  nextRepeatMode,
  radioSectionStart,
  repeatModeLabel,
} from "@parson/music-sdk";
import { RadioQueueSection, RadioThumbs } from "./radio/radio-controls";
import {
  ListMusic,
  ListX,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  Trash2,
  X,
} from "lucide-react";
import Image from "next/image";
import {
  Fragment,
  type DragEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import { queueDropIndex } from "./player-queue-reorder";
import { queueSummary } from "./queue-summary";
import { SongTitle, ExplicitBadge } from "@/features/library/song-title";
import { isExplicitSongTitle } from "@/features/library/song-presentation";
import { trackArtistCredit } from "@/features/library/track-artist-credit";

/**
 * Queue management for the local player or a remote zone (another browser,
 * the phone, a Sonos room). Edits go through the same callbacks either way;
 * for a remote zone they become zone commands.
 */
export type QueueEditing = {
  /** Index of the playing item; only items after it can be edited. */
  currentIndex: number;
  editable: boolean;
  /** The local player can also reorder history and the playing track. */
  reorderAnywhere?: boolean;
  onRemove: (index: number) => void;
  onClearUpcoming: () => void;
  repeatMode: ZoneRepeatMode;
  /** `null` when the zone cannot repeat. */
  onRepeatChange: ((mode: ZoneRepeatMode) => void) | null;
  shuffle: boolean;
  /** `null` when the zone cannot shuffle. */
  onShuffleChange: ((enabled: boolean) => void) | null;
};

export function canEditQueueRow(
  editing: QueueEditing | undefined,
  index: number,
  length: number,
) {
  return Boolean(
    editing?.editable && index > editing.currentIndex && index < length,
  );
}

export function QueueToolbar({
  editing,
  length,
}: {
  editing: QueueEditing;
  length: number;
}) {
  const upcoming = Math.max(0, length - editing.currentIndex - 1);
  const RepeatIcon = editing.repeatMode === "one" ? Repeat1 : Repeat;
  const toggleClass = (active: boolean) =>
    `flex h-8 w-8 items-center justify-center rounded-full transition-colors disabled:opacity-40 ${
      active
        ? "bg-mujin-copper/[0.16] text-mujin-copper"
        : "text-mujin-text-tertiary hover:bg-white/[0.07] hover:text-mujin-text"
    }`;
  return (
    <div className="flex items-center gap-1">
      {editing.onShuffleChange && (
        <button
          aria-label={editing.shuffle ? "Shuffle on" : "Shuffle off"}
          aria-pressed={editing.shuffle}
          className={toggleClass(editing.shuffle)}
          onClick={() => editing.onShuffleChange?.(!editing.shuffle)}
          title={
            editing.shuffle
              ? "Shuffle on: turn off to restore the order"
              : "Shuffle upcoming"
          }
          type="button"
        >
          <Shuffle className="h-4 w-4" />
        </button>
      )}
      {editing.onRepeatChange && (
        <button
          aria-label={repeatModeLabel(editing.repeatMode)}
          aria-pressed={editing.repeatMode !== "off"}
          className={toggleClass(editing.repeatMode !== "off")}
          onClick={() =>
            editing.onRepeatChange?.(nextRepeatMode(editing.repeatMode))
          }
          title={repeatModeLabel(editing.repeatMode)}
          type="button"
        >
          <RepeatIcon className="h-4 w-4" />
        </button>
      )}
      {editing.editable && (
        <button
          aria-label="Clear upcoming"
          className="flex h-8 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium text-zinc-400 transition-colors hover:bg-white/10 hover:text-white disabled:pointer-events-none disabled:opacity-40"
          disabled={upcoming === 0}
          onClick={editing.onClearUpcoming}
          title={`Remove the ${upcoming} upcoming ${upcoming === 1 ? "track" : "tracks"}`}
          type="button"
        >
          <ListX className="h-4 w-4" />
          Clear
        </button>
      )}
    </div>
  );
}

export default function PlayerQueue({
  currentSongId,
  editing,
  onClose,
  onReorder,
  onSelect,
  queue,
  title = "Queue",
}: {
  currentSongId: string;
  editing?: QueueEditing;
  onClose: () => void;
  onReorder: (from: number, to: number) => void;
  onSelect: (index: number) => void;
  queue: Player["queue"];
  title?: string;
}) {
  const queueRef = useRef<HTMLElement>(null);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node) || queueRef.current?.contains(target)) {
        return;
      }
      if (
        target instanceof Element &&
        target.closest("[data-player-queue-trigger]")
      ) {
        return;
      }
      onClose();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        onClose();
        document
          .querySelector<HTMLButtonElement>(
            "[data-player-queue-trigger] button",
          )
          ?.focus();
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return (
    <aside
      aria-label="Queue"
      className="fixed bottom-[calc(var(--shell-footer)+80px)] left-2 right-2 z-60 flex max-h-[min(460px,calc(100dvh-220px))] flex-col overflow-hidden rounded-xl border border-white/[0.08] bg-mujin-raised/97 text-mujin-text shadow-2xl backdrop-blur-2xl md:inset-x-auto md:bottom-(--shell-footer) md:right-0 md:top-(--shell-header) md:max-h-none md:w-[380px] md:rounded-none md:border-y-0 md:border-r-0 md:border-l md:border-white/[0.06] md:shadow-[-30px_0_60px_-30px_rgba(0,0,0,0.9)] motion-safe:animate-in motion-safe:fade-in motion-safe:md:slide-in-from-right-4"
      ref={queueRef}
    >
      <header className="flex items-center justify-between gap-2 border-b border-white/[0.06] py-3 pl-5 pr-3">
        <div className="min-w-0">
          <h2 className="mj-title min-w-0 truncate text-[22px] leading-tight">
            {title}
          </h2>
          <p className="mj-tnum text-[11.5px] text-mujin-text-tertiary">
            {queueSummary(queue, editing?.currentIndex)}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {editing && <QueueToolbar editing={editing} length={queue.length} />}
          <button
            aria-label="Close queue"
            className="flex h-8 w-8 items-center justify-center rounded-full text-zinc-500 hover:bg-white/10 hover:text-white"
            onClick={onClose}
            type="button"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </header>
      {queue.length ? (
        <PlayerQueueList
          currentSongId={currentSongId}
          editing={editing}
          onReorder={onReorder}
          onSelect={onSelect}
          queue={queue}
        />
      ) : (
        <div className="flex flex-1 flex-col overflow-y-auto p-2">
          <div className="grid flex-1 place-items-center px-6 py-6 text-center">
            <div>
              <ListMusic className="mx-auto h-7 w-7 text-mujin-text-disabled" />
              <p className="mj-title mt-3 text-lg text-mujin-text">
                Go find something to play
              </p>
              <p className="mt-1 text-xs leading-5 text-mujin-text-tertiary">
                Play a song or album, or start a station.
              </p>
            </div>
          </div>
          <RadioQueueSection hasPicks={false} />
        </div>
      )}
    </aside>
  );
}

export function PlayerQueueList({
  className = "",
  currentSongId,
  editing,
  onReorder,
  onSelect,
  queue,
}: {
  className?: string;
  currentSongId: string;
  /** When given, only upcoming rows drag, and rows get a remove button. */
  editing?: QueueEditing;
  onReorder: (from: number, to: number) => void;
  onSelect: (index: number) => void;
  queue: Player["queue"];
}) {
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);
  const [dropTarget, setDropTarget] = useState<{
    edge: "before" | "after";
    index: number;
  } | null>(null);
  const suppressClick = useRef(false);
  const { currentIndex } = usePlayer();
  // A remote zone's queue is positioned by the zone, not this player.
  const radioStart = radioSectionStart(
    queue.map((item) => item.origin),
    editing?.currentIndex ?? currentIndex,
  );

  const movable = (index: number) =>
    !editing ||
    Boolean(editing.reorderAnywhere) ||
    canEditQueueRow(editing, index, queue.length);
  const move = (from: number, to: number) => {
    if (from === to || from < 0 || to < 0 || to >= queue.length) {
      return;
    }
    if (!movable(from) || !movable(to)) {
      return;
    }
    onReorder(from, to);
  };
  const drop = (event: DragEvent<HTMLButtonElement>, targetIndex: number) => {
    event.preventDefault();
    const sourceIndex =
      draggedIndex ?? Number(event.dataTransfer.getData("text/plain"));
    const bounds = event.currentTarget.getBoundingClientRect();
    const edge =
      event.clientY < bounds.top + bounds.height / 2 ? "before" : "after";
    move(
      sourceIndex,
      queueDropIndex(queue.length, sourceIndex, targetIndex, edge),
    );
    setDraggedIndex(null);
    setDropTarget(null);
  };
  const moveWithKeyboard = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") {
      return;
    }
    event.preventDefault();
    move(index, index + (event.key === "ArrowUp" ? -1 : 1));
  };

  return (
    <div
      aria-label="Playback queue. Drag rows to reorder, or use the arrow keys."
      className={`overflow-y-auto p-2 ${className}`}
    >
      {queue.map((item, index) => {
        const active = editing
          ? index === editing.currentIndex
          : item.song.id === currentSongId;
        const rowMovable = movable(index);
        const removable = canEditQueueRow(editing, index, queue.length);
        const radioPick = radioStart >= 0 && index >= radioStart;
        const cover =
          getLibraryImageUrl(item.album.cover_url, getBaseURL) ?? defaultCover;
        return (
          <Fragment key={`${item.song.id}-${index}`}>
            {index === radioStart && <RadioQueueSection hasPicks />}
            <div className="group/row relative">
              <button
                aria-current={active ? "true" : undefined}
                aria-label={`${item.song.name || "Untitled song"}. Position ${index + 1} of ${queue.length}.${rowMovable ? " Drag to reorder, or use the arrow keys." : ""}`}
                className={`group relative flex h-15.5 w-full items-center gap-3 rounded-xl p-2 text-left transition-colors ${
                  rowMovable ? "cursor-grab active:cursor-grabbing" : ""
                } ${active ? "bg-white/9" : "hover:bg-white/5"} ${
                  draggedIndex === index ? "opacity-60" : ""
                } ${
                  radioPick
                    ? removable
                      ? "pr-32"
                      : "pr-22"
                    : removable
                      ? "pr-11"
                      : ""
                }`}
                data-native-drag="true"
                draggable={rowMovable}
                onClick={() => {
                  if (!suppressClick.current) {
                    onSelect(index);
                  }
                }}
                onDragEnd={() => {
                  setDraggedIndex(null);
                  setDropTarget(null);
                  window.setTimeout(() => {
                    suppressClick.current = false;
                  }, 0);
                }}
                onDragOver={(event) => {
                  if (draggedIndex === null || !rowMovable) {
                    return;
                  }
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  const bounds = event.currentTarget.getBoundingClientRect();
                  setDropTarget({
                    edge:
                      event.clientY < bounds.top + bounds.height / 2
                        ? "before"
                        : "after",
                    index,
                  });
                }}
                onDragStart={(event) => {
                  if (!rowMovable) {
                    event.preventDefault();
                    return;
                  }
                  suppressClick.current = true;
                  setDraggedIndex(index);
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", String(index));
                }}
                onDrop={(event) => drop(event, index)}
                onKeyDown={(event) => moveWithKeyboard(event, index)}
                type="button"
              >
                {dropTarget?.index === index && draggedIndex !== index && (
                  <span
                    aria-hidden="true"
                    className={`pointer-events-none absolute inset-x-2 z-20 h-0.5 rounded-full bg-white ${
                      dropTarget.edge === "before" ? "top-0" : "bottom-0"
                    }`}
                  />
                )}
                <div className="relative h-11 w-11 shrink-0 overflow-hidden rounded-md bg-zinc-900">
                  <Image
                    alt=""
                    className="object-cover"
                    draggable={false}
                    fill
                    sizes="44px"
                    src={cover}
                  />
                  <span className="absolute inset-0 hidden place-items-center bg-black/50 group-hover:grid">
                    <Play className="h-4 w-4 fill-white" />
                  </span>
                </div>
                <div className="min-w-0 flex-1">
                  <p
                    className={`truncate text-sm font-medium ${active ? "text-white" : "text-zinc-200"}`}
                  >
                    <SongTitle title={item.song.name || "Untitled song"} />
                  </p>
                  <p className="truncate text-xs text-zinc-500">
                    {radioPick && item.reason && (
                      <span className="sr-only">
                        Radio pick, {item.reason}.{" "}
                      </span>
                    )}
                    {isExplicitSongTitle(item.song.name) && <ExplicitBadge />}
                    {trackArtistCredit({
                      albumArtist: item.artist.name,
                      albumType: item.album.primary_type,
                      contributingArtists: item.song.contributing_artists ?? [],
                      trackArtist: item.song.artist,
                    }) ||
                      item.artist.name ||
                      item.song.artist ||
                      "Unknown artist"}
                  </p>
                </div>
                {active && (
                  <span className="mr-2 h-2 w-2 rounded-full bg-white" />
                )}
              </button>
              {radioPick && (
                <RadioThumbs
                  className={`absolute top-1/2 -translate-y-1/2 ${removable ? "right-11" : "right-2"}`}
                  index={index}
                  label={item.song.name || "this song"}
                  liked={item.liked}
                  size="sm"
                />
              )}
              {removable && editing && (
                <button
                  aria-label={`Remove ${item.song.name || "this song"} from the queue`}
                  className="absolute right-2 top-1/2 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-full text-zinc-500 opacity-0 transition hover:bg-white/10 hover:text-white focus-visible:opacity-100 group-hover/row:opacity-100 pointer-coarse:opacity-100"
                  onClick={() => editing.onRemove(index)}
                  title="Remove from queue"
                  type="button"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              )}
            </div>
          </Fragment>
        );
      })}
      {radioStart < 0 && <RadioQueueSection hasPicks={false} />}
    </div>
  );
}
