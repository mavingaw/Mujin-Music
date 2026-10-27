"use client";

import { Button } from "@/components/ui/button";
import {
  reanalyzeLibraryItems,
  reidentifyLibraryItems,
  type Breakdown,
  type LibraryToolsOverview,
  type ToolJob,
} from "@parson/music-sdk";
import {
  AlertTriangle,
  AudioWaveform,
  ChevronRight,
  Copy,
  Disc3,
  FileWarning,
  FolderX,
  Layers,
  Loader2,
  ScanSearch,
} from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { ToolCard } from "./tool-primitives";
import {
  barSegments,
  formatBytes,
  formatCount,
  formatPlayingTime,
  isQualityTier,
  losslessShare,
  percent,
  runningJob,
  tierColors,
  tierShortLabels,
  type ToolView,
} from "./tools-model";
import { useStartToolJob } from "./use-tool-jobs";

export function OverviewPanel({
  overview,
  loading,
  jobs,
  onNavigate,
}: {
  overview: LibraryToolsOverview | undefined;
  loading: boolean;
  jobs: ToolJob[];
  onNavigate: (view: ToolView) => void;
}) {
  const startJob = useStartToolJob();
  if (loading || !overview) {
    return (
      <div className="flex h-64 items-center justify-center text-zinc-500">
        <Loader2
          aria-label="Loading library statistics"
          className="h-5 w-5 animate-spin"
        />
      </div>
    );
  }
  const { stats } = overview;
  const share = losslessShare(stats.tiers);
  const identifying = runningJob(jobs, "reidentify");
  const analyzing = runningJob(jobs, "reanalyze");
  const identified = stats.identification;
  const identifiedTotal =
    identified.identified + identified.awaiting + identified.unidentified;
  const analysisTotal =
    stats.analysis.analyzed + stats.analysis.partial + stats.analysis.awaiting;

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        <Stat label="Tracks" value={formatCount(stats.tracks)} />
        <Stat label="Albums" value={formatCount(stats.albums)} />
        <Stat label="Artists" value={formatCount(stats.artists)} />
        <Stat
          label="Playing time"
          value={formatPlayingTime(stats.duration_seconds)}
        />
        <Stat label="On disk" value={formatBytes(stats.size_bytes)} />
      </div>

      <ToolCard
        description="Every track by the resolution of the file that plays."
        title={
          <span className="flex items-baseline gap-3">
            Quality
            <span className="text-sm font-normal text-zinc-500">
              <span className="font-semibold text-mujin-copper-hover">
                {share}%
              </span>{" "}
              lossless
            </span>
          </span>
        }
      >
        <QualityBar tiers={stats.tiers} />
        <div className="mt-6 grid gap-6 md:grid-cols-3">
          <BreakdownList
            items={stats.formats}
            title="Formats"
            total={stats.tracks}
          />
          <BreakdownList
            items={stats.sample_rates}
            title="Sample rates (lossless)"
            total={stats.sample_rates.reduce(
              (sum, item) => sum + item.tracks,
              0,
            )}
          />
          <BreakdownList
            items={stats.bit_depths}
            title="Bit depths (PCM)"
            total={stats.bit_depths.reduce((sum, item) => sum + item.tracks, 0)}
          />
        </div>
      </ToolCard>

      <div className="grid gap-6 lg:grid-cols-2">
        <ToolCard
          action={
            <Button
              disabled={Boolean(identifying) || !identified.enabled}
              onClick={() =>
                void startJob(
                  () => reidentifyLibraryItems({ scope: "library" }),
                  "Could not start identification.",
                )
              }
              size="sm"
              variant="secondary"
            >
              {identifying ? (
                <Loader2 className="animate-spin" />
              ) : (
                <ScanSearch />
              )}
              Re-identify
            </Button>
          }
          description="Albums matched to a MusicBrainz release get credits, release dates and artwork."
          title="Identification"
        >
          <Meter
            segments={[
              {
                label: "Identified",
                value: identified.identified,
                color: "var(--color-mujin-copper)",
              },
              {
                label: "Awaiting lookup",
                value: identified.awaiting,
                color: "var(--color-quality-enhanced)",
              },
              {
                label: "Not identified",
                value: identified.unidentified,
                color: "var(--color-mujin-surface-3)",
              },
            ]}
            total={identifiedTotal}
            unit="tracks"
          />
          {!identified.enabled && (
            <p className="mt-4 text-xs leading-5 text-zinc-500">
              Online metadata is off, so albums use their file tags.{" "}
              <Link
                className="text-zinc-300 underline-offset-4 hover:underline"
                href="/settings?tab=library"
              >
                Turn it on in Settings
              </Link>{" "}
              to identify them.
            </p>
          )}
        </ToolCard>

        <ToolCard
          action={
            <Button
              disabled={Boolean(analyzing)}
              onClick={() =>
                void startJob(
                  () => reanalyzeLibraryItems({ scope: "library" }),
                  "Could not start the analysis.",
                )
              }
              size="sm"
              variant="secondary"
            >
              {analyzing ? (
                <Loader2 className="animate-spin" />
              ) : (
                <AudioWaveform />
              )}
              Analyze new tracks
            </Button>
          }
          description="EBU R128 loudness for volume leveling, waveforms, and acoustic fingerprints that confirm duplicates by sound."
          title="Audio analysis"
        >
          <Meter
            segments={[
              {
                label: "Analyzed",
                value: stats.analysis.analyzed,
                color: "var(--color-quality-high)",
              },
              {
                label: "Partly",
                value: stats.analysis.partial,
                color: "var(--color-quality-enhanced)",
              },
              {
                label: "Not yet",
                value: stats.analysis.awaiting,
                color: "var(--color-mujin-surface-3)",
              },
            ]}
            total={analysisTotal}
            unit="tracks"
          />
        </ToolCard>
      </div>

      <div>
        <h2 className="mb-3 text-[15px] font-semibold text-white">
          Needs attention
        </h2>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <AttentionTile
            count={overview.skipped}
            description={
              overview.skipped_checked_at
                ? "Files in your folders that are not in the library"
                : "Check your folders for files that didn't import"
            }
            icon={<FileWarning />}
            label="Skipped files"
            onClick={() => onNavigate("skipped")}
            tone={overview.skipped ? "warn" : "ok"}
          />
          <AttentionTile
            count={overview.duplicates}
            description={
              overview.duplicates_suggested
                ? `${formatCount(overview.duplicates_suggested)} can be tidied in one step`
                : "Same recording held more than once"
            }
            icon={<Copy />}
            label="Duplicates"
            onClick={() => onNavigate("duplicates")}
            tone={overview.duplicates_suggested ? "warn" : "neutral"}
          />
          <AttentionTile
            count={overview.version_groups}
            description="Remasters and editions grouped under one album"
            icon={<Layers />}
            label="Album versions"
            onClick={() => onNavigate("versions")}
            tone="neutral"
          />
          <AttentionTile
            count={overview.loose_tracks}
            description="Tracks alone on an album of one, ready to group"
            icon={<Disc3 />}
            label="Loose tracks"
            onClick={() => onNavigate("albumizer")}
            tone={overview.loose_tracks ? "warn" : "ok"}
          />
          <AttentionTile
            count={overview.missing}
            description="Files the library remembers but can't find"
            icon={<FolderX />}
            label="Missing files"
            onClick={() => onNavigate("maintenance")}
            tone={overview.missing ? "warn" : "ok"}
          />
          <AttentionTile
            count={overview.multi_copy_tracks}
            description="Tracks with several files; the best one plays"
            icon={<AlertTriangle />}
            label="Multiple copies"
            onClick={() => onNavigate("duplicates")}
            tone="neutral"
          />
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-white/[0.07] bg-white/[0.025] px-4 py-3.5">
      <p className="text-[11px] font-medium uppercase tracking-[0.14em] text-zinc-500">
        {label}
      </p>
      <p className="mt-1 truncate text-2xl font-semibold tabular-nums tracking-tight text-white">
        {value}
      </p>
    </div>
  );
}

function QualityBar({ tiers }: { tiers: Breakdown[] }) {
  const segments = barSegments(tiers);
  const total = tiers.reduce((sum, tier) => sum + tier.tracks, 0);
  if (!segments.length) {
    return <p className="text-sm text-zinc-500">No tracks yet.</p>;
  }
  return (
    <div>
      <div
        aria-label="Tracks by quality"
        className="flex h-3 overflow-hidden rounded-full bg-white/[0.04]"
        role="img"
      >
        {segments.map((segment) => {
          const color = isQualityTier(segment.key)
            ? tierColors[segment.key]
            : "var(--color-mujin-text-disabled)";
          return (
            <div
              className="h-full first:rounded-l-full last:rounded-r-full"
              key={segment.key}
              style={{
                width: `${segment.width}%`,
                background: `linear-gradient(180deg, ${color}, ${color}cc)`,
                boxShadow: `0 0 12px ${color}55`,
              }}
              title={`${segment.label}: ${formatCount(segment.tracks)}`}
            />
          );
        })}
      </div>
      <div className="mt-3 flex flex-wrap gap-x-5 gap-y-2">
        {segments.map((segment) => (
          <span
            className="flex items-center gap-2 text-xs text-zinc-400"
            key={segment.key}
          >
            <span
              className="h-2 w-2 rounded-full"
              style={{
                backgroundColor: isQualityTier(segment.key)
                  ? tierColors[segment.key]
                  : "var(--color-mujin-text-disabled)",
              }}
            />
            <span className="text-zinc-200">
              {isQualityTier(segment.key)
                ? tierShortLabels[segment.key]
                : segment.label}
            </span>
            <span className="tabular-nums">
              {formatCount(segment.tracks)} · {percent(segment.tracks, total)}%
            </span>
          </span>
        ))}
      </div>
    </div>
  );
}

function BreakdownList({
  title,
  items,
  total,
}: {
  title: string;
  items: Breakdown[];
  total: number;
}) {
  return (
    <div>
      <h3 className="mb-2.5 text-xs font-medium uppercase tracking-[0.14em] text-zinc-500">
        {title}
      </h3>
      {items.length === 0 ? (
        <p className="text-sm text-zinc-600">None</p>
      ) : (
        <ul className="space-y-2">
          {items.slice(0, 8).map((item) => (
            <li className="text-sm" key={item.key}>
              <div className="flex items-baseline justify-between gap-3">
                <span className="truncate text-zinc-200">{item.label}</span>
                <span className="shrink-0 text-xs tabular-nums text-zinc-500">
                  {formatCount(item.tracks)}
                </span>
              </div>
              <div className="mt-1 h-1 overflow-hidden rounded-full bg-white/[0.05]">
                <div
                  className="h-full rounded-full bg-zinc-400/70"
                  style={{
                    width: `${Math.max(2, percent(item.tracks, total))}%`,
                  }}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Meter({
  segments,
  total,
  unit,
}: {
  segments: { label: string; value: number; color: string }[];
  total: number;
  unit: string;
}) {
  return (
    <div>
      <div className="flex h-2 overflow-hidden rounded-full bg-white/[0.04]">
        {segments.map((segment) =>
          segment.value > 0 ? (
            <div
              key={segment.label}
              style={{
                width: `${Math.max(1, (segment.value / Math.max(1, total)) * 100)}%`,
                backgroundColor: segment.color,
              }}
            />
          ) : null,
        )}
      </div>
      <dl className="mt-4 grid grid-cols-3 gap-3">
        {segments.map((segment) => (
          <div key={segment.label}>
            <dt className="flex items-center gap-1.5 text-xs text-zinc-500">
              <span
                className="h-1.5 w-1.5 rounded-full"
                style={{ backgroundColor: segment.color }}
              />
              {segment.label}
            </dt>
            <dd className="mt-0.5 text-lg font-semibold tabular-nums text-white">
              {formatCount(segment.value)}
              <span className="ml-1 text-xs font-normal text-zinc-600">
                {percent(segment.value, total)}%
              </span>
            </dd>
          </div>
        ))}
      </dl>
      <p className="sr-only">
        {formatCount(total)} {unit}
      </p>
    </div>
  );
}

function AttentionTile({
  label,
  count,
  description,
  icon,
  tone,
  onClick,
}: {
  label: string;
  count: number;
  description: string;
  icon: ReactNode;
  tone: "ok" | "warn" | "neutral";
  onClick: () => void;
}) {
  const accent =
    tone === "warn"
      ? "#daab6c"
      : tone === "ok"
        ? "var(--color-quality-high)"
        : "var(--color-mujin-text-secondary)";
  return (
    <button
      className="group flex items-center gap-4 rounded-xl border border-white/[0.07] bg-white/[0.02] px-4 py-4 text-left transition-colors hover:border-white/[0.14] hover:bg-white/[0.045]"
      onClick={onClick}
      type="button"
    >
      <span
        className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg [&_svg]:h-5 [&_svg]:w-5"
        style={{ backgroundColor: `${accent}14`, color: accent }}
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="text-sm font-medium text-zinc-100">{label}</span>
          <span className="text-sm font-semibold tabular-nums text-white">
            {formatCount(count)}
          </span>
        </span>
        <span className="mt-0.5 block truncate text-xs text-zinc-500">
          {description}
        </span>
      </span>
      <ChevronRight className="h-4 w-4 shrink-0 text-zinc-600 transition-colors group-hover:text-zinc-300" />
    </button>
  );
}
