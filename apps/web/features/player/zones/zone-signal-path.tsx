"use client";

import {
  streamQuality,
  streamQualityText,
  type Zone,
  type ZoneStreamQuality,
} from "@parson/music-sdk";
import {
  ArrowDownToLine,
  AudioWaveform,
  BadgeCheck,
  Binary,
  FileAudio,
  Gauge,
  Package,
  RefreshCcw,
  SlidersHorizontal,
  Speaker,
  type LucideIcon,
} from "lucide-react";
import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  signalPathCaveat,
  signalPathStages,
  type SignalStageIcon,
} from "./zone-signal-path-model";

/** The Mujin Music quality light: copper, blue, green, dimmed grey. */
export const QUALITY_TONES: Record<
  ZoneStreamQuality,
  { dot: string; text: string; border: string; line: string }
> = {
  lossless: {
    dot: "bg-quality-lossless shadow-[0_0_10px_2px_rgba(217,139,95,0.55)]",
    text: "text-quality-lossless",
    border: "border-quality-lossless/30",
    line: "from-quality-lossless/70",
  },
  enhanced: {
    dot: "bg-quality-enhanced shadow-[0_0_10px_2px_rgba(143,195,224,0.5)]",
    text: "text-quality-enhanced",
    border: "border-quality-enhanced/30",
    line: "from-quality-enhanced/70",
  },
  high: {
    dot: "bg-quality-high shadow-[0_0_10px_2px_rgba(163,201,139,0.45)]",
    text: "text-quality-high",
    border: "border-quality-high/30",
    line: "from-quality-high/70",
  },
  low: {
    dot: "bg-quality-low",
    text: "text-quality-low",
    border: "border-quality-low/30",
    line: "from-quality-low/70",
  },
};

export function QualityLight({
  quality,
  className = "",
}: {
  quality: ZoneStreamQuality;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={`inline-block h-2 w-2 shrink-0 rounded-full ${QUALITY_TONES[quality].dot} ${className}`}
    />
  );
}

const STAGE_ICONS: Record<SignalStageIcon, LucideIcon> = {
  source: ArrowDownToLine,
  rate: AudioWaveform,
  depth: Binary,
  gain: Gauge,
  dsp: SlidersHorizontal,
  decode: FileAudio,
  encode: Package,
  sync: RefreshCcw,
  output: Speaker,
};

/** Source, each processing step in order, then the output. */
export function SignalPathStages({ zone }: { zone: Zone }) {
  const stream = zone.output?.stream;
  if (!stream) return null;
  const quality = streamQuality(stream);
  const tone = QUALITY_TONES[quality];
  const stages = signalPathStages(stream);
  return (
    <ol className="relative" data-signal-path>
      <span
        aria-hidden
        className={`absolute bottom-3 left-[13px] top-3 w-px bg-gradient-to-b ${tone.line} to-white/10`}
      />
      {stages.map((stage, index) => {
        const Icon = STAGE_ICONS[stage.icon];
        const strong = stage.kicker !== null;
        return (
          <li
            className="relative flex items-start gap-3 py-1.5"
            data-signal-stage={stage.kicker?.toLowerCase() ?? "step"}
            key={`${index}-${stage.label}`}
          >
            <span
              className={`relative z-10 flex h-[27px] w-[27px] shrink-0 items-center justify-center rounded-full border bg-zinc-950 ${strong ? `${tone.border} ${tone.text}` : "border-white/10 text-zinc-400"}`}
            >
              <Icon className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 pt-0.5">
              {stage.kicker && (
                <span className="block text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                  {stage.kicker}
                </span>
              )}
              <span
                className={`block text-[13px] leading-5 ${strong ? "font-medium text-white" : "text-zinc-300"}`}
              >
                {stage.label}
              </span>
            </span>
          </li>
        );
      })}
    </ol>
  );
}

/** The full panel: rating, stages, the zone's format note and caveats. */
export function SignalPathPanel({ zone }: { zone: Zone }) {
  const output = zone.output;
  const stream = output?.stream;
  if (!output) return null;
  if (!stream) {
    return (
      <div className="space-y-2">
        <p className="text-[13px] text-zinc-400">
          Nothing from Mujin Music is playing here right now.
        </p>
        <p className="text-[11px] leading-4 text-zinc-500">{output.note}</p>
      </div>
    );
  }
  const quality = streamQuality(stream);
  const text = streamQualityText(quality);
  const tone = QUALITY_TONES[quality];
  const caveat = signalPathCaveat(zone);
  return (
    <div className="space-y-4" data-quality={quality}>
      <div
        className={`flex items-start gap-3 rounded-xl border bg-white/[0.03] p-3 ${tone.border}`}
      >
        <QualityLight className="mt-1.5" quality={quality} />
        <div className="min-w-0">
          <p className={`text-sm font-semibold ${tone.text}`}>
            {text.label}
            {stream.bit_perfect && (
              <span className="ml-2 inline-flex items-center gap-1 rounded-full border border-quality-lossless/30 px-1.5 py-px align-middle text-[10px] font-medium text-quality-lossless">
                <BadgeCheck className="h-3 w-3" />
                Bit-perfect
              </span>
            )}
          </p>
          <p className="mt-0.5 text-xs leading-4 text-zinc-400">
            {stream.bit_perfect
              ? "Every sample of the file reaches the output untouched."
              : text.description}
          </p>
        </div>
      </div>
      <SignalPathStages zone={zone} />
      <div className="space-y-1.5 border-t border-white/[0.06] pt-3">
        <p className="text-[11px] leading-4 text-zinc-400">{output.note}</p>
        {caveat && (
          <p className="text-[11px] leading-4 text-zinc-500">{caveat}</p>
        )}
      </div>
    </div>
  );
}

export function ZoneSignalPathDialog({
  onOpenChange,
  open,
  zone,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  zone: Zone;
}) {
  const stream = zone.output?.stream;
  const label = stream ? streamQualityText(streamQuality(stream)).label : null;
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Signal path{label ? `: ${label}` : ""}</DialogTitle>
          <DialogDescription>
            {zone.name}
            {zone.now_playing ? ` · ${zone.now_playing.title}` : ""}
          </DialogDescription>
        </DialogHeader>
        <SignalPathPanel zone={zone} />
      </DialogContent>
    </Dialog>
  );
}

/**
 * The quality light: a coloured dot and its label (`compact`: the label only
 * on wide screens) that opens the zone's signal path. Nothing when the zone
 * has no path to show.
 */
export function ZoneQualityLight({
  compact = false,
  zone,
}: {
  compact?: boolean;
  zone: Zone;
}) {
  const [open, setOpen] = useState(false);
  const stream = zone.output?.stream;
  if (!stream) return null;
  const quality = streamQuality(stream);
  const text = streamQualityText(quality);
  return (
    <>
      <button
        aria-label={`Signal path: ${text.label}`}
        className={`relative z-30 inline-flex items-center gap-1.5 rounded-full border border-white/10 px-2 py-0.5 text-[11px] font-medium transition hover:border-white/25 hover:bg-white/[0.06] ${QUALITY_TONES[quality].text}`}
        data-quality-light={quality}
        onClick={(event) => {
          event.stopPropagation();
          setOpen(true);
        }}
        title={`Signal path: ${text.label}`}
        type="button"
      >
        <QualityLight quality={quality} />
        <span className={compact ? "hidden lg:inline" : undefined}>
          {stream.bit_perfect ? "Bit-perfect" : text.label}
        </span>
      </button>
      <ZoneSignalPathDialog onOpenChange={setOpen} open={open} zone={zone} />
    </>
  );
}
