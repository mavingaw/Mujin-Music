"use client";

import { useSession } from "@/features/account/session-provider";
import { cancelToolJob, getLibraryToolsOverview } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { ShieldCheck } from "lucide-react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { AlbumizerPanel } from "./albumizer-panel";
import { DuplicatesPanel } from "./duplicates-panel";
import { MaintenancePanel } from "./maintenance-panel";
import { OverviewPanel } from "./overview-panel";
import { SkippedPanel } from "./skipped-panel";
import { CountPill, EmptyState, JobProgress } from "./tool-primitives";
import {
  parseToolView,
  toolViewLabels,
  toolViews,
  type ToolView,
} from "./tools-model";
import { useToolJobWatcher } from "./use-tool-jobs";
import { VersionsPanel } from "./versions-panel";

export const overviewKey = ["library-tools", "overview"] as const;

export default function LibraryToolsPage() {
  const { session } = useSession();
  const router = useRouter();
  const pathname = usePathname();
  const view = parseToolView(useSearchParams().get("view"));
  const isAdmin = session?.role === "admin";
  const jobs = useToolJobWatcher();
  const overview = useQuery({
    queryKey: overviewKey,
    queryFn: getLibraryToolsOverview,
    enabled: isAdmin,
  });

  if (session && !isAdmin) {
    return (
      <div className="mx-auto max-w-3xl px-5 py-16">
        <EmptyState icon={<ShieldCheck />} title="Library tools are for admins">
          Ask the person who runs this Mujin Music server to tidy the library.
        </EmptyState>
      </div>
    );
  }

  const counts: Partial<Record<ToolView, number>> = overview.data
    ? {
        skipped: overview.data.skipped,
        duplicates: overview.data.duplicates,
        versions: overview.data.version_groups,
        albumizer: overview.data.loose_tracks,
        maintenance: overview.data.missing,
      }
    : {};
  const running = (jobs.data ?? []).filter((job) => job.state === "running");
  const select = (next: ToolView) => {
    const params = new URLSearchParams();
    if (next !== "overview") params.set("view", next);
    const query = params.toString();
    router.replace(query ? `${pathname}?${query}` : pathname, {
      scroll: false,
    });
  };

  return (
    <div className="mx-auto w-full max-w-[1180px] px-5 py-9 pb-36 sm:px-7">
      <header className="mb-7 flex flex-wrap items-end justify-between gap-4">
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-mujin-copper">
            Library
          </p>
          <h1 className="mt-1 text-3xl font-semibold tracking-tight text-white sm:text-4xl">
            Library tools
          </h1>
          <p className="mt-2 max-w-xl text-sm leading-6 text-zinc-500">
            Keep your collection complete, tidy and in its best quality. Mujin
            Music only reads your music files; every change here can be undone.
          </p>
        </div>
      </header>

      {running.length > 0 && (
        <div className="mb-6 grid gap-2 md:grid-cols-2">
          {running.map((job) => (
            <JobProgress
              job={job}
              key={job.id}
              onCancel={() => void cancelToolJob(job.id)}
            />
          ))}
        </div>
      )}

      <nav
        aria-label="Library tools"
        className="app-shell-scroll -mx-1 mb-8 flex gap-1 overflow-x-auto border-b border-white/[0.07] px-1"
        role="tablist"
      >
        {toolViews.map((item) => (
          <button
            aria-selected={view === item}
            className={`relative flex shrink-0 items-center px-3 pb-3 pt-1 text-sm font-medium transition-colors ${
              view === item ? "text-white" : "text-zinc-500 hover:text-zinc-200"
            }`}
            key={item}
            onClick={() => select(item)}
            role="tab"
            type="button"
          >
            {toolViewLabels[item]}
            <CountPill count={counts[item] ?? 0} />
            {view === item && (
              <span className="absolute inset-x-3 -bottom-px h-0.5 rounded-full bg-[linear-gradient(90deg,#7c4dff,#b26bff)]" />
            )}
          </button>
        ))}
      </nav>

      <div role="tabpanel">
        {view === "overview" && (
          <OverviewPanel
            jobs={jobs.data ?? []}
            onNavigate={select}
            overview={overview.data}
            loading={overview.isPending}
          />
        )}
        {view === "skipped" && <SkippedPanel jobs={jobs.data ?? []} />}
        {view === "duplicates" && <DuplicatesPanel jobs={jobs.data ?? []} />}
        {view === "versions" && <VersionsPanel />}
        {view === "albumizer" && <AlbumizerPanel />}
        {view === "maintenance" && (
          <MaintenancePanel
            jobs={jobs.data ?? []}
            lastScanAt={overview.data?.last_scan_at ?? null}
          />
        )}
      </div>
    </div>
  );
}
