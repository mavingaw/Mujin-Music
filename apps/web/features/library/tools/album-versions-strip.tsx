"use client";

import { useSession } from "@/features/account/session-provider";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import { cn } from "@/lib/cn";
import { chooseAlbumVersion, getAlbumVersions } from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Layers, Star } from "lucide-react";
import Link from "next/link";
import { toast } from "sonner";
import { getApiErrorMessage } from "./tool-errors";
import { QualityBadge, ToolCover } from "./tool-primitives";

/**
 * Other versions of this album in the library (remasters, editions), with
 * the preferred one marked. Admins only, like the rest of library tools.
 */
export function AlbumVersionsStrip({ albumId }: { albumId: string }) {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const isAdmin = session?.role === "admin";
  const versions = useQuery({
    queryKey: ["library-tools", "album-versions", albumId],
    queryFn: () => getAlbumVersions(albumId),
    enabled: isAdmin && Boolean(albumId),
    staleTime: 60_000,
  });
  const group = versions.data;
  if (!isAdmin || !group || group.versions.length < 2) return null;

  const prefer = async (preferredId: string) => {
    try {
      await chooseAlbumVersion(group.key, preferredId, group.hide_others);
      toast.success("Preferred version changed");
      await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
      await invalidateCatalogRevisionQueries(queryClient);
    } catch (error) {
      toast(
        getApiErrorMessage(error, "Could not change the preferred version."),
      );
    }
  };

  return (
    <section aria-labelledby="album-versions-heading" className="mt-12">
      <div className="mb-4 flex items-baseline justify-between gap-3">
        <h2
          className="flex items-center gap-2 text-lg font-semibold text-white"
          id="album-versions-heading"
        >
          <Layers className="h-4 w-4 text-[#b26bff]" />
          Versions in your library
        </h2>
        <Link
          className="text-xs text-zinc-500 transition-colors hover:text-white"
          href="/library/tools?view=versions"
        >
          Manage versions
        </Link>
      </div>
      <div className="app-shell-scroll -mx-1 flex gap-3 overflow-x-auto px-1 pb-2">
        {group.versions.map((version) => {
          const current = version.album_id === albumId;
          const tile = (
            <>
              <div className="relative">
                <ToolCover
                  alt={version.title}
                  className="!h-auto !w-full aspect-square"
                  size={150}
                  src={version.cover_url}
                />
                {version.preferred && (
                  <span className="absolute left-2 top-2 flex items-center gap-1 rounded-full bg-black/75 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-[#d6b8ff] backdrop-blur">
                    <Star className="h-3 w-3 fill-current" />
                    Preferred
                  </span>
                )}
              </div>
              <p className="mt-2 truncate text-sm font-medium text-zinc-100">
                {version.edition}
              </p>
              <p className="truncate text-xs text-zinc-500">
                {[version.year, `${version.track_count} tracks`]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
              <div className="mt-1.5">
                <QualityBadge quality={version.quality} />
              </div>
            </>
          );
          return (
            <div
              className={cn(
                "w-[150px] shrink-0 rounded-lg p-2 transition-colors",
                current
                  ? "bg-white/[0.06] ring-1 ring-white/15"
                  : "hover:bg-white/[0.04]",
              )}
              key={version.album_id}
            >
              {current || version.hidden ? (
                <div>{tile}</div>
              ) : (
                <Link
                  href={`/album?id=${encodeURIComponent(version.album_id)}`}
                >
                  {tile}
                </Link>
              )}
              {current && (
                <p className="mt-1 text-[11px] text-zinc-400">This version</p>
              )}
              {!version.preferred && (
                <button
                  className="mt-1 text-[11px] text-zinc-500 underline-offset-4 hover:text-white hover:underline"
                  onClick={() => void prefer(version.album_id)}
                  type="button"
                >
                  {version.hidden
                    ? "Hidden · prefer this"
                    : "Prefer this version"}
                </button>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
