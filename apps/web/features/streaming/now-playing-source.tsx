"use client";

import { getStreamingLibrary, parseStreamingId } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { LibraryButton } from "./library-button";
import { ProviderMark } from "./source-badge";

/**
 * Player controls for a streaming track: where it plays from, and the
 * library toggle that stands in for the local favorite heart.
 */
export function NowPlayingStreamingSource({
  songId,
  title,
}: {
  songId: string;
  title: string;
}) {
  const parsed = parseStreamingId(songId);
  const library = useQuery({
    queryKey: ["streaming", "library", "track", "all"],
    queryFn: () => getStreamingLibrary({ kind: "track", limit: 500 }),
    enabled: Boolean(parsed),
    staleTime: 30_000,
  });
  if (!parsed) return null;
  const inLibrary = Boolean(
    library.data?.items.some((item) => item.id === songId),
  );
  return (
    <span className="flex shrink-0 items-center gap-1">
      <ProviderMark className="ml-1" provider={parsed.provider} />
      <LibraryButton
        compact
        id={songId}
        inLibrary={inLibrary}
        kind="track"
        title={title}
      />
    </span>
  );
}
