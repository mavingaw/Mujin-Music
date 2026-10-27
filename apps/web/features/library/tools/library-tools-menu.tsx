"use client";

import {
  ContextMenuItem,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import { useSession } from "@/features/account/session-provider";
import {
  reanalyzeLibraryItems,
  reidentifyLibraryItems,
  rescanLibraryItems,
  type ToolJob,
  type ToolScope,
} from "@parson/music-sdk";
import { AudioWaveform, RefreshCcw, ScanSearch, Wrench } from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { useStartToolJob } from "./use-tool-jobs";

/**
 * "Library tools" submenu for albums and tracks (admins only): rescan the
 * files, identify the album again, analyze the audio. Each runs as a job
 * with progress on the Library tools page.
 */
export function LibraryToolsSubmenu({
  scope,
  onShowAnalysis,
}: {
  scope: Extract<ToolScope, { scope: "albums" | "songs" }>;
  onShowAnalysis?: () => void;
}) {
  const { session } = useSession();
  const startJob = useStartToolJob();
  const router = useRouter();
  if (session?.role !== "admin") return null;

  const run = async (start: () => Promise<ToolJob>, failure: string) => {
    const job = await startJob(start, failure);
    if (job) {
      toast.success(`${job.title}: ${job.scope}`, {
        description: "Follow its progress in Library tools.",
        action: {
          label: "Open",
          onClick: () => router.push("/library/tools?view=maintenance"),
        },
      });
    }
  };

  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <Wrench className="h-4 w-4" />
        Library tools
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="w-56">
        {onShowAnalysis && (
          <ContextMenuItem onSelect={onShowAnalysis}>
            <AudioWaveform className="h-4 w-4" />
            Audio analysis…
          </ContextMenuItem>
        )}
        <ContextMenuItem
          onSelect={() =>
            void run(
              () => rescanLibraryItems(scope),
              "Could not start the rescan.",
            )
          }
        >
          <RefreshCcw className="h-4 w-4" />
          Rescan files
        </ContextMenuItem>
        {scope.scope === "albums" && (
          <ContextMenuItem
            onSelect={() =>
              void run(
                () => reidentifyLibraryItems(scope),
                "Could not identify this album.",
              )
            }
          >
            <ScanSearch className="h-4 w-4" />
            Re-identify album
          </ContextMenuItem>
        )}
        <ContextMenuItem
          onSelect={() =>
            void run(
              () => reanalyzeLibraryItems(scope, true),
              "Could not start the analysis.",
            )
          }
        >
          <AudioWaveform className="h-4 w-4" />
          Re-analyze audio
        </ContextMenuItem>
        <ContextMenuItem asChild>
          <Link href="/library/tools">
            <Wrench className="h-4 w-4" />
            Open Library tools
          </Link>
        </ContextMenuItem>
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}
