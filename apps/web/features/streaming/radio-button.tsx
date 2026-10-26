"use client";

import {
  createStreamingPlaylistCopy,
  describeRadio,
  getStreamingRadio,
  parseStreamingId,
  type StreamingRadioSeed,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, Radio } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/cn";
import { radioPlaylistTitle } from "./streaming-model";
import { streamingErrorMessage, useStreamingPlayback } from "./use-streaming";

/**
 * Starts a radio from a TIDAL or Qobuz track, artist or album, plays it and
 * offers to keep it as a playlist in Parson.
 */
export function useStreamingRadio() {
  const playback = useStreamingPlayback();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      seed,
      id,
    }: {
      seed: StreamingRadioSeed;
      id: string;
      label: string;
    }) => getStreamingRadio(seed, id),
    onSuccess: (radio, { id, label }) => {
      const parsed = parseStreamingId(id);
      if (!parsed) return;
      const playable = radio.tracks.filter((track) => track.streamable);
      if (!playable.length) {
        toast("This radio has nothing playable right now.");
        return;
      }
      playback.playTracks(playable);
      const title = radioPlaylistTitle(label);
      toast.success(`${describeRadio(parsed.provider, radio)}: ${label}`, {
        description: `${playable.length} tracks queued.`,
        action: {
          label: "Keep as playlist",
          onClick: () => {
            void createStreamingPlaylistCopy(parsed.provider, {
              title,
              description: `${describeRadio(parsed.provider, radio)}, started from ${label}.`,
              track_ids: playable.map((track) => track.id),
            })
              .then(async (copy) => {
                toast.success(`Saved ${copy.title} in Parson`);
                await queryClient.invalidateQueries({
                  queryKey: ["streaming", "playlist-copies"],
                });
              })
              .catch((error: unknown) =>
                toast(
                  streamingErrorMessage(error, "Could not save the radio."),
                ),
              );
          },
        },
      });
    },
    onError: (error) =>
      toast(streamingErrorMessage(error, "Could not start a radio.")),
  });
}

export function StartRadioButton({
  seed,
  id,
  label,
  compact = false,
  className,
}: {
  seed: StreamingRadioSeed;
  id: string;
  /** What the radio starts from, e.g. a track or artist name. */
  label: string;
  compact?: boolean;
  className?: string;
}) {
  const radio = useStreamingRadio();
  const text = `Start radio from ${label}`;
  return (
    <button
      aria-label={text}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-full text-sm font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/30 disabled:opacity-60",
        compact
          ? "h-8 w-8 text-zinc-400 hover:bg-white/10 hover:text-white"
          : "h-10 border border-white/15 px-4 text-white hover:bg-white/6",
        className,
      )}
      disabled={radio.isPending}
      onClick={(event) => {
        event.stopPropagation();
        radio.mutate({ seed, id, label });
      }}
      title={text}
      type="button"
    >
      {radio.isPending ? (
        <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
      ) : (
        <Radio aria-hidden="true" className="h-4 w-4" />
      )}
      {!compact && "Start radio"}
    </button>
  );
}
