"use client";

import { useSession } from "@/features/account/session-provider";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import { getApiErrorMessage } from "@/features/library/tools/tool-errors";
import {
  enhancedStatusLabel,
  getAlbumUpgradeStatus,
  isUpgradeOpen,
  playOriginalsForAlbum,
  upgradeProgressDetail,
  upgradeStatusLabel,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Loader2, Sparkles } from "lucide-react";
import Link from "next/link";
import { toast } from "sonner";

/**
 * Better versions on the album page: a lossless copy on its way from FETCH
 * ("Lossless version on its way · 45%", "Handed to Lidarr"), and whether
 * AI-enhanced copies play, with the original one click away. Everyone sees
 * the status; only admins can switch.
 */
export function AlbumUpgradeStatus({ albumId }: { albumId: string }) {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const isAdmin = session?.role === "admin";
  const status = useQuery({
    queryKey: ["upgrades", "album", albumId],
    queryFn: () => getAlbumUpgradeStatus(albumId),
    enabled: Boolean(albumId),
    staleTime: 5_000,
    refetchInterval: (query) =>
      query.state.data?.request && isUpgradeOpen(query.state.data.request.state)
        ? 5_000
        : false,
  });
  const data = status.data;
  if (!data) return null;
  const request = data.request;
  const showRequest =
    request && request.state !== "cancelled" && request.state !== "not_found";
  const playingEnhanced = data.enhanced_tracks > 0;
  const hasEnhanced = data.enhanced_copies > 0;
  if (!showRequest && !hasEnhanced && data.enhance_queued === 0) return null;

  const switchCopies = async (originals: boolean) => {
    try {
      await playOriginalsForAlbum(albumId, originals);
      toast.success(
        originals ? "The originals play now" : "AI-enhanced copies play again",
      );
      await queryClient.invalidateQueries({ queryKey: ["upgrades"] });
      await invalidateCatalogRevisionQueries(queryClient);
    } catch (error) {
      toast(getApiErrorMessage(error, "Could not switch the copies."));
    }
  };

  const open = request ? isUpgradeOpen(request.state) : false;
  const progress =
    request?.state === "downloading" && request.progress
      ? Math.max(2, Math.min(100, request.progress))
      : null;
  return (
    <div className="mt-3 flex flex-wrap items-center gap-2" data-album-upgrade>
      {showRequest && request && (
        <span
          className="inline-flex max-w-full items-center gap-2 rounded-full border border-mujin-copper/30 bg-mujin-copper/[0.08] py-1 pl-2.5 pr-3 text-xs font-medium text-mujin-copper-hover"
          data-request-state={request.state}
          title={request.message || upgradeProgressDetail(request) || undefined}
        >
          {open ? (
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
          ) : (
            <Download className="h-3.5 w-3.5 shrink-0" />
          )}
          <span className="truncate">{upgradeStatusLabel(request)}</span>
          {progress !== null && (
            <span className="hidden h-1 w-20 overflow-hidden rounded-full bg-white/10 sm:block">
              <span
                className="block h-full rounded-full bg-mujin-copper transition-[width] duration-500"
                style={{ width: `${progress}%` }}
              />
            </span>
          )}
          {open && request.eta && (
            <span className="hidden font-normal text-zinc-400 sm:inline">
              {request.eta} left
            </span>
          )}
        </span>
      )}
      {hasEnhanced && (
        <span
          className="inline-flex items-center gap-1.5 rounded-full border border-quality-enhanced/35 bg-quality-enhanced/[0.08] py-1 pl-2.5 pr-3 text-xs font-medium text-quality-enhanced"
          data-ai-enhanced
          title="Restored by the AI enhancer: plausible treble, not the original master."
        >
          <Sparkles className="h-3.5 w-3.5" />
          {enhancedStatusLabel(data)}
        </span>
      )}
      {hasEnhanced && isAdmin && (
        <button
          className="text-xs text-zinc-400 underline-offset-2 hover:text-white hover:underline"
          onClick={() => void switchCopies(playingEnhanced)}
          type="button"
        >
          {playingEnhanced ? "Play the originals" : "Play AI-enhanced"}
        </button>
      )}
      {data.enhance_queued > 0 && (
        <span className="inline-flex items-center gap-1.5 text-xs text-quality-enhanced">
          <Loader2 className="h-3 w-3 animate-spin" />
          {data.enhance_queued} track(s) being enhanced
        </span>
      )}
      {isAdmin && (showRequest || hasEnhanced) && (
        <Link
          className="text-xs text-zinc-500 hover:text-white"
          href="/library/tools?view=better"
        >
          Better versions
        </Link>
      )}
    </div>
  );
}
