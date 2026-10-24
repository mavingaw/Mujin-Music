"use client";

import type { StreamingItemKind } from "@parson/music-sdk";
import { Check, Loader2, Plus } from "lucide-react";
import { cn } from "@/lib/cn";
import { useStreamingLibraryToggle } from "./use-streaming";

/** Roon's "+ Add to library": adds to Parson and favorites on the service. */
export function LibraryButton({
  kind,
  id,
  title,
  inLibrary,
  compact = false,
  className,
}: {
  kind: StreamingItemKind;
  id: string;
  title: string;
  inLibrary: boolean;
  compact?: boolean;
  className?: string;
}) {
  const toggle = useStreamingLibraryToggle();
  const pending = toggle.isPending;
  const showInLibrary = pending ? !inLibrary : inLibrary;
  const label = showInLibrary
    ? `Remove ${title} from library`
    : `Add ${title} to library`;
  return (
    <button
      aria-label={label}
      aria-pressed={showInLibrary}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-full text-sm font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/30 disabled:opacity-60",
        compact
          ? "h-8 w-8 text-zinc-400 hover:bg-white/10 hover:text-white"
          : showInLibrary
            ? "h-10 border border-white/15 px-4 text-zinc-200 hover:border-white/30"
            : "h-10 border border-white/25 px-4 text-white hover:bg-white/8",
        compact && showInLibrary && "text-emerald-300",
        className,
      )}
      disabled={pending}
      onClick={(event) => {
        event.stopPropagation();
        toggle.mutate({ kind, id, add: !inLibrary, title });
      }}
      title={label}
      type="button"
    >
      {pending ? (
        <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
      ) : showInLibrary ? (
        <Check aria-hidden="true" className="h-4 w-4" />
      ) : (
        <Plus aria-hidden="true" className="h-4 w-4" />
      )}
      {!compact && (showInLibrary ? "In library" : "Add to library")}
    </button>
  );
}
