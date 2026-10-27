"use client";

import { cn } from "@/lib/cn";
import getBaseURL from "@/lib/api/server-url";
import { defaultCover } from "@/lib/images/default-cover";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import type { FileQuality, ToolJob } from "@parson/music-sdk";
import { toolJobProgress } from "@parson/music-sdk";
import { Loader2, X } from "lucide-react";
import Image from "next/image";
import type { ReactNode } from "react";
import { jobProgressLabel, tierColors } from "./tools-model";

/** A file's quality as a pill coloured like the signal-path light. */
export function QualityBadge({
  quality,
  className,
  emphasis = false,
}: {
  quality: FileQuality;
  className?: string;
  emphasis?: boolean;
}) {
  const color = tierColors[quality.tier];
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-1.5 rounded-full border px-2 text-[11px] font-medium tabular-nums tracking-wide",
        className,
      )}
      data-quality-tier={quality.tier}
      style={{
        borderColor: `${color}55`,
        backgroundColor: emphasis ? `${color}26` : `${color}12`,
        color,
      }}
      title={quality.label}
    >
      <span
        aria-hidden
        className="h-1.5 w-1.5 rounded-full"
        style={{
          backgroundColor: color,
          boxShadow: `0 0 6px ${color}`,
        }}
      />
      {quality.label}
    </span>
  );
}

export function ToolCover({
  src,
  alt,
  className,
  size = 48,
}: {
  src: string;
  alt: string;
  className?: string;
  size?: number;
}) {
  const url = getLibraryImageUrl(src, getBaseURL) ?? defaultCover;
  return (
    <span
      className={cn(
        "relative block shrink-0 overflow-hidden rounded-md bg-mujin-raised shadow-[inset_0_0_0_1px_rgba(255,255,255,0.06)]",
        className,
      )}
      style={{ width: size, height: size }}
    >
      <Image
        alt={alt}
        className="h-full w-full object-cover"
        height={size * 2}
        onError={(event) => {
          event.currentTarget.src = defaultCover;
        }}
        src={url}
        width={size * 2}
      />
    </span>
  );
}

/** A panel card. */
export function ToolCard({
  title,
  description,
  action,
  children,
  className,
}: {
  title?: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn(
        "rounded-xl border border-white/[0.07] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0.012))] p-5",
        className,
      )}
    >
      {(title || action) && (
        <div className="mb-4 flex flex-wrap items-start justify-between gap-3 sm:flex-nowrap">
          <div className="min-w-0 flex-1">
            {title && (
              <h2 className="text-[15px] font-semibold text-white">{title}</h2>
            )}
            {description && (
              <p className="mt-1 text-sm leading-5 text-zinc-500">
                {description}
              </p>
            )}
          </div>
          {action && <div className="flex shrink-0 gap-2">{action}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

/** Live progress of one job, with a cancel button while it runs. */
export function JobProgress({
  job,
  onCancel,
  compact = false,
}: {
  job: ToolJob;
  onCancel?: () => void;
  compact?: boolean;
}) {
  const progress = toolJobProgress(job);
  const running = job.state === "running";
  const failed = job.state === "failed";
  return (
    <div
      aria-live="polite"
      className={cn(
        "rounded-lg border border-white/[0.07] bg-black/30",
        compact ? "px-3 py-2" : "px-4 py-3",
      )}
      data-job-state={job.state}
    >
      <div className="flex items-center gap-3">
        {running && (
          <Loader2 className="h-4 w-4 shrink-0 animate-spin text-mujin-copper" />
        )}
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-zinc-100">
            {job.title}
            <span className="font-normal text-zinc-500"> · {job.scope}</span>
          </p>
          <p
            className={cn(
              "truncate text-xs",
              failed ? "text-red-300" : "text-zinc-500",
            )}
          >
            {running && job.current
              ? `${jobProgressLabel(job)} · ${job.current}`
              : jobProgressLabel(job)}
            {job.failed > 0 && running ? ` · ${job.failed} failed` : ""}
          </p>
        </div>
        {running && onCancel && (
          <button
            aria-label={`Stop ${job.title}`}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-zinc-500 transition-colors hover:bg-white/8 hover:text-white"
            onClick={onCancel}
            type="button"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>
      {running && (
        <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-white/[0.06]">
          {progress === null ? (
            <div className="h-full w-1/3 animate-[tool-progress_1.4s_ease-in-out_infinite] rounded-full bg-[linear-gradient(90deg,transparent,var(--color-mujin-copper),transparent)]" />
          ) : (
            <div
              className="h-full rounded-full bg-[linear-gradient(90deg,var(--color-mujin-copper-pressed),var(--color-mujin-copper))] transition-[width] duration-500"
              style={{ width: `${Math.max(2, progress * 100)}%` }}
            />
          )}
        </div>
      )}
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  children,
}: {
  icon: ReactNode;
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-white/[0.08] px-6 py-14 text-center">
      <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-white/[0.04] text-zinc-400 [&_svg]:h-5 [&_svg]:w-5">
        {icon}
      </div>
      <p className="text-sm font-medium text-zinc-200">{title}</p>
      {children && (
        <div className="mt-1 max-w-md text-sm text-zinc-500">{children}</div>
      )}
    </div>
  );
}

export function CountPill({ count }: { count: number }) {
  if (!count) return null;
  return (
    <span className="ml-1.5 inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-white/[0.08] px-1.5 text-[10px] font-semibold tabular-nums text-zinc-300">
      {count > 999 ? "999+" : count}
    </span>
  );
}
