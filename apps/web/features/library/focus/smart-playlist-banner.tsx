"use client";

import {
  detachSmartPlaylist,
  encodeFocus,
  focusSummary,
  refreshSmartPlaylist,
  type SmartPlaylistInfo,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Link2Off,
  ListFilter,
  Loader2,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useRef } from "react";
import { toast } from "sonner";
import { smartRefreshAge, smartRefreshLabel } from "./focus-state";

/** Refreshes on open when older than this, so the playlist follows the library. */
const AUTO_REFRESH_AFTER_MS = 60_000;

export default function SmartPlaylistBanner({
  playlistId,
  smart,
}: {
  playlistId: number;
  smart: SmartPlaylistInfo;
}) {
  const queryClient = useQueryClient();
  const invalidate = async () => {
    await queryClient.invalidateQueries({ queryKey: ["playlist", playlistId] });
    await queryClient.invalidateQueries({ queryKey: ["playlists"] });
  };
  const refresh = useMutation({
    mutationFn: () => refreshSmartPlaylist(playlistId),
    onSuccess: invalidate,
    onError: () => toast("Could not refresh the smart playlist."),
  });
  const detach = useMutation({
    mutationFn: () => detachSmartPlaylist(playlistId),
    onSuccess: async () => {
      toast.success("This playlist no longer follows its focus.");
      await invalidate();
    },
    onError: () => toast("Could not change the playlist."),
  });
  const autoRefreshed = useRef(false);
  useEffect(() => {
    if (autoRefreshed.current) return;
    autoRefreshed.current = true;
    if (
      smartRefreshAge(smart.refreshed_at, Date.now()) > AUTO_REFRESH_AFTER_MS
    ) {
      refresh.mutate();
    }
    // Only on first open of this playlist.
  }, [refresh, smart.refreshed_at]);

  const summary = focusSummary(smart.query);
  const libraryHref = `/library?${new URLSearchParams({
    ...(smart.query.entity === "albums" ? {} : { view: smart.query.entity }),
    focus: encodeFocus(smart.query),
  })}`;

  return (
    <div className="mt-6 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-white/8 bg-white/[0.03] px-4 py-3 text-sm">
      <Sparkles
        aria-hidden="true"
        className="h-4 w-4 shrink-0 text-amber-200"
      />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-zinc-100">Smart playlist</p>
        <p className="truncate text-xs text-zinc-500">
          {summary || "Every song in your library"} ·{" "}
          {smartRefreshLabel(smart.refreshed_at, Date.now())}
        </p>
      </div>
      <div className="flex items-center gap-1">
        <button
          className="flex h-8 items-center gap-1.5 rounded-full px-3 text-zinc-300 hover:bg-white/8 hover:text-white disabled:opacity-50"
          disabled={refresh.isPending}
          onClick={() => refresh.mutate()}
          type="button"
        >
          {refresh.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
          Refresh
        </button>
        <Link
          className="flex h-8 items-center gap-1.5 rounded-full px-3 text-zinc-300 hover:bg-white/8 hover:text-white"
          href={libraryHref}
        >
          <ListFilter className="h-4 w-4" />
          Open focus
        </Link>
        <button
          className="flex h-8 items-center gap-1.5 rounded-full px-3 text-zinc-500 hover:bg-white/8 hover:text-white disabled:opacity-50"
          disabled={detach.isPending}
          onClick={() => detach.mutate()}
          title="Keep the current songs and stop following the focus"
          type="button"
        >
          <Link2Off className="h-4 w-4" />
          Stop updating
        </button>
      </div>
    </div>
  );
}
