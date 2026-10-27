"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  getSongAnalysis,
  reanalyzeLibraryItems,
  rescanLibraryItems,
  type SongAnalysis,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { AudioWaveform, Fingerprint, Loader2, RefreshCcw } from "lucide-react";
import { QualityBadge, ToolCover } from "./tool-primitives";
import {
  formatBytes,
  formatServerTime,
  formatTrackTime,
  runningJob,
} from "./tools-model";
import { useStartToolJob, useToolJobs } from "./use-tool-jobs";

/** Mirrored peak/RMS waveform, drawn as an SVG that scales to any width. */
export function Waveform({
  peaks,
  rms,
  height = 72,
}: {
  peaks: number[];
  rms: number[];
  height?: number;
}) {
  const count = peaks.length;
  if (!count) return null;
  const middle = height / 2;
  const step = 1000 / count;
  const bar = Math.max(1, step * 0.62);
  return (
    <svg
      aria-label="Waveform"
      className="block w-full"
      height={height}
      preserveAspectRatio="none"
      role="img"
      viewBox={`0 0 1000 ${height}`}
      width="100%"
    >
      <defs>
        <linearGradient id="waveform-peak" x1="0" x2="1" y1="0" y2="0">
          <stop offset="0" stopColor="#7c4dff" />
          <stop offset="1" stopColor="#d6b8ff" />
        </linearGradient>
      </defs>
      {peaks.map((peak, index) => {
        const peakHeight = Math.max(1, (peak / 255) * (middle - 1));
        const rmsHeight = Math.max(
          0.5,
          ((rms[index] ?? 0) / 255) * (middle - 1),
        );
        const x = index * step + (step - bar) / 2;
        return (
          <g key={index}>
            <rect
              fill="url(#waveform-peak)"
              height={peakHeight * 2}
              opacity={0.35}
              rx={bar / 2}
              width={bar}
              x={x}
              y={middle - peakHeight}
            />
            <rect
              fill="url(#waveform-peak)"
              height={rmsHeight * 2}
              rx={bar / 2}
              width={bar}
              x={x}
              y={middle - rmsHeight}
            />
          </g>
        );
      })}
    </svg>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-[11px] uppercase tracking-[0.12em] text-zinc-500">
        {label}
      </dt>
      <dd className="mt-0.5 text-sm tabular-nums text-zinc-100">{value}</dd>
    </div>
  );
}

function signed(value: number | undefined, unit: string) {
  if (value === undefined || !Number.isFinite(value)) return "—";
  return `${value > 0 ? "+" : ""}${value.toFixed(1)} ${unit}`;
}

function AnalysisBody({ analysis }: { analysis: SongAnalysis }) {
  const { song } = analysis;
  return (
    <div className="min-w-0 space-y-5">
      <div className="flex items-center gap-3">
        <ToolCover alt={song.album} size={56} src={song.cover_url} />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-white">
            {song.title}
          </p>
          <p className="truncate text-xs text-zinc-500">
            {song.artist} · {song.album}
          </p>
        </div>
        <QualityBadge emphasis quality={song.quality} />
      </div>

      <div className="rounded-lg border border-white/[0.06] bg-black/40 px-3 py-4">
        {analysis.waveform ? (
          <Waveform
            peaks={analysis.waveform.peaks}
            rms={analysis.waveform.rms}
          />
        ) : (
          <p className="py-5 text-center text-xs text-zinc-500">
            {analysis.analysis_error
              ? `Analysis failed: ${analysis.analysis_error}`
              : "Not analyzed yet."}
          </p>
        )}
        <div className="mt-2 flex justify-between text-[10px] tabular-nums text-zinc-600">
          <span>0:00</span>
          <span>{formatTrackTime(song.duration)}</span>
        </div>
      </div>

      <dl className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Fact
          label="Loudness"
          value={
            analysis.loudness
              ? `${analysis.loudness.integrated_lufs.toFixed(1)} LUFS`
              : "—"
          }
        />
        <Fact
          label="True peak"
          value={
            analysis.loudness
              ? `${analysis.loudness.true_peak_db.toFixed(1)} dBTP`
              : "—"
          }
        />
        <Fact
          label="ReplayGain"
          value={signed(analysis.replay_gain?.track_gain_db, "dB")}
        />
        <Fact label="File size" value={formatBytes(analysis.size_bytes)} />
      </dl>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-zinc-500">
        <span className="flex items-center gap-1.5">
          <Fingerprint className="h-3.5 w-3.5" />
          {analysis.fingerprint === "chromaprint"
            ? "Chromaprint fingerprint"
            : analysis.fingerprint === "envelope"
              ? "Loudness-envelope fingerprint"
              : "No fingerprint yet"}
        </span>
        {analysis.analyzed_at && (
          <span>Analyzed {formatServerTime(analysis.analyzed_at)}</span>
        )}
        {analysis.stale && (
          <span className="text-[#f5c542]">The file changed since</span>
        )}
      </div>

      {analysis.copies.length > 1 && (
        <div>
          <p className="mb-2 text-xs text-zinc-400">Copies of this track</p>
          <ul className="space-y-1">
            {analysis.copies.map((copy) => (
              <li
                className="flex items-center gap-3 rounded-md border border-white/[0.06] px-3 py-1.5"
                key={copy.path}
              >
                <QualityBadge quality={copy.quality} />
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-500">
                  {copy.path}
                </span>
                {copy.playing && (
                  <span className="text-[11px] text-mujin-copper-hover">
                    Playing
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      <p
        className="truncate font-mono text-[11px] text-zinc-600"
        title={song.path}
      >
        {song.path}
      </p>
    </div>
  );
}

export function SongAnalysisDialog({
  songId,
  open,
  onOpenChange,
}: {
  songId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const startJob = useStartToolJob();
  const jobs = useToolJobs();
  const analyzing = runningJob(jobs.data, "reanalyze");
  const analysis = useQuery({
    queryKey: ["library-tools", "song-analysis", songId],
    queryFn: () => getSongAnalysis(songId),
    enabled: open,
    refetchInterval: analyzing ? 2000 : false,
  });
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl [&>*]:min-w-0">
        <DialogHeader>
          <DialogTitle>Audio analysis</DialogTitle>
          <DialogDescription>
            Loudness, waveform and fingerprint of the file that plays.
          </DialogDescription>
        </DialogHeader>
        {analysis.isPending ? (
          <div className="flex h-40 items-center justify-center text-zinc-500">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : analysis.data ? (
          <AnalysisBody analysis={analysis.data} />
        ) : (
          <p className="text-sm text-zinc-500">
            The analysis could not be loaded.
          </p>
        )}
        <div className="flex flex-wrap justify-end gap-2">
          <Button
            onClick={() =>
              void startJob(
                () => rescanLibraryItems({ scope: "songs", ids: [songId] }),
                "Could not rescan this file.",
              )
            }
            size="sm"
            variant="ghost"
          >
            <RefreshCcw />
            Rescan file
          </Button>
          <Button
            disabled={Boolean(analyzing)}
            onClick={() =>
              void startJob(
                () =>
                  reanalyzeLibraryItems(
                    { scope: "songs", ids: [songId] },
                    true,
                  ),
                "Could not analyze this track.",
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
            Analyze again
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
