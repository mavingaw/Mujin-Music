"use client";

import {
  describeCollectionFormat,
  signalPathSummary,
  signalQualityColors,
  signalQualityLabels,
  type SignalPath,
  type SignalQuality,
  type SignalStage,
  type SignalStageKind,
  type VolumeLevelingMode,
  type AudioProperties,
} from "@parson/music-sdk";
import {
  ArrowLeftRight,
  AudioWaveform,
  Cpu,
  FileAudio,
  Gauge,
  type LucideIcon,
  Server,
  Settings2,
  SlidersHorizontal,
  Sparkles,
  Speaker,
  Volume2,
  X,
} from "lucide-react";
import Link from "next/link";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/cn";
import {
  volumeLevelingModeLabels,
  volumeLevelingModes,
} from "./volume-leveling";

const STAGE_ICONS: Record<SignalStageKind, LucideIcon> = {
  source: FileAudio,
  dsp: Sparkles,
  conversion: ArrowLeftRight,
  encode: AudioWaveform,
  transport: Server,
  decode: Cpu,
  volume: Volume2,
  output: Speaker,
};

const QUALITY_ORDER: SignalQuality[] = [
  "lossless",
  "enhanced",
  "high_quality",
  "low_quality",
];

/** Roon's light: a sparkle for lossless and enhanced, a dot otherwise. */
export function QualityGlyph({
  quality,
  size = 14,
  className,
}: {
  quality: SignalQuality | null;
  size?: number;
  className?: string;
}) {
  const color = quality ? signalQualityColors[quality] : "#71717a";
  const sparkle = quality === "lossless" || quality === "enhanced";
  return (
    <svg
      aria-hidden="true"
      className={className}
      height={size}
      viewBox="0 0 16 16"
      width={size}
      style={{
        color,
        filter: quality ? `drop-shadow(0 0 4px ${color}aa)` : undefined,
      }}
    >
      {sparkle ? (
        <path
          d="M8 0.8 9.5 6.5 15.2 8 9.5 9.5 8 15.2 6.5 9.5 0.8 8 6.5 6.5Z"
          fill="currentColor"
        />
      ) : (
        <circle cx="8" cy="8" fill="currentColor" r="3.6" />
      )}
    </svg>
  );
}

function qualityText(path: SignalPath) {
  return signalQualityLabels[path.quality];
}

type PopoverProps = {
  casting: boolean;
  levelingMode: VolumeLevelingMode;
  loading?: boolean;
  onLevelingModeChange: (mode: VolumeLevelingMode) => void;
  /** Opens the DSP engine, as Roon's signal path opens MUSE. */
  onOpenDsp?: () => void;
  path: SignalPath | null;
  streamBitrate: number;
};

/**
 * The quality light. `compact` is the footer light; `pill` adds the
 * quality and format for the now-playing view.
 */
export function SignalPathLight({
  variant = "compact",
  className,
  ...props
}: PopoverProps & { variant?: "compact" | "pill"; className?: string }) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelId = useId();
  const { path } = props;
  const label = path
    ? `Signal path: ${qualityText(path)}${path.highRes ? ", high resolution" : ""}`
    : "Signal path";

  return (
    <>
      <button
        aria-controls={open ? panelId : undefined}
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={label}
        className={cn(
          variant === "compact"
            ? "flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors hover:bg-white/8"
            : "inline-flex h-8 max-w-full items-center gap-2 rounded-full border border-white/10 bg-white/4 pl-2.5 pr-3 text-xs text-zinc-300 transition-colors hover:bg-white/8",
          className,
        )}
        data-signal-quality={path?.quality ?? "unknown"}
        onClick={() => setOpen((value) => !value)}
        ref={triggerRef}
        title={variant === "compact" ? label : undefined}
        type="button"
      >
        <QualityGlyph
          quality={path?.quality ?? null}
          size={variant === "compact" ? 15 : 13}
        />
        {variant === "pill" && path ? (
          <>
            <span
              className="shrink-0 font-semibold"
              style={{ color: signalQualityColors[path.quality] }}
            >
              {qualityText(path)}
            </span>
            {path.highRes ? <HiResBadge /> : null}
            <span className="truncate text-zinc-400">
              {path.source?.summary ?? "Format unknown"}
            </span>
          </>
        ) : null}
      </button>
      {open ? (
        <SignalPathPanel
          {...props}
          anchor={triggerRef}
          id={panelId}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}

function HiResBadge() {
  return (
    <span className="shrink-0 rounded-[5px] border border-current px-1 text-[9px] font-bold leading-3.5 tracking-wide text-[#c9a2ff]">
      HI-RES
    </span>
  );
}

function usePanelPosition(
  anchor: React.RefObject<HTMLElement | null>,
  panel: React.RefObject<HTMLElement | null>,
) {
  const [style, setStyle] = useState<React.CSSProperties>({
    visibility: "hidden",
  });
  const place = useCallback(() => {
    const trigger = anchor.current?.getBoundingClientRect();
    const element = panel.current;
    if (!trigger || !element) return;
    const margin = 12;
    const width = Math.min(400, window.innerWidth - margin * 2);
    const height = element.offsetHeight;
    const left = Math.min(
      Math.max(margin, trigger.left + trigger.width / 2 - width / 2),
      window.innerWidth - width - margin,
    );
    const above = trigger.top - margin - height;
    const top =
      above >= margin
        ? above
        : Math.min(
            trigger.bottom + margin,
            window.innerHeight - height - margin,
          );
    setStyle({ left, top: Math.max(margin, top), width });
  }, [anchor, panel]);
  useLayoutEffect(() => {
    place();
    const element = panel.current;
    const observer = element ? new ResizeObserver(place) : null;
    if (element) observer?.observe(element);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [place, panel]);
  return style;
}

function SignalPathPanel({
  anchor,
  casting,
  id,
  levelingMode,
  loading,
  onClose,
  onLevelingModeChange,
  onOpenDsp,
  path,
  streamBitrate,
}: PopoverProps & {
  anchor: React.RefObject<HTMLButtonElement | null>;
  id: string;
  onClose: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const style = usePanelPosition(anchor, panelRef);
  const [selected, setSelected] = useState<string | null>(null);
  const titleId = `${id}-title`;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      onClose();
      anchor.current?.focus();
    };
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node;
      if (
        panelRef.current?.contains(target) ||
        anchor.current?.contains(target)
      )
        return;
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("pointerdown", onPointer, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", onPointer, true);
    };
  }, [anchor, onClose]);

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  if (typeof document === "undefined") return null;
  const quality = path?.quality ?? null;

  return createPortal(
    <div
      aria-labelledby={titleId}
      className="fixed z-120 flex max-h-[min(78dvh,680px)] flex-col overflow-hidden rounded-2xl border border-white/10 bg-[#0d0d0f]/98 text-left text-white shadow-[0_24px_80px_rgba(0,0,0,0.75)] outline-none backdrop-blur-xl"
      data-signal-path-panel
      id={id}
      onContextMenu={(event) => event.stopPropagation()}
      ref={panelRef}
      role="dialog"
      style={style}
      tabIndex={-1}
    >
      <header className="flex items-start gap-3 border-b border-white/6 px-5 pb-3 pt-4">
        <div className="min-w-0 flex-1">
          <h2 className="text-[17px] font-semibold leading-6" id={titleId}>
            Signal path:{" "}
            <span
              style={{
                color: quality ? signalQualityColors[quality] : undefined,
              }}
            >
              {path ? qualityText(path) : loading ? "Checking" : "Unknown"}
            </span>
            {path?.highRes ? (
              <span className="ml-2 inline-block align-middle">
                <HiResBadge />
              </span>
            ) : null}
          </h2>
          <p className="mt-0.5 text-xs leading-5 text-zinc-500">
            {path
              ? `${signalPathSummary(path)}. Select a stage to learn more.`
              : "Select a stage to learn more."}
          </p>
          {path?.bitPerfectToOutput && path.quality !== "lossless" ? (
            <p
              className="mt-1.5 flex items-center gap-1.5 text-xs font-medium"
              style={{ color: signalQualityColors.lossless }}
            >
              <QualityGlyph quality="lossless" size={10} />
              Bit-perfect up to the {casting ? "Cast device" : "system mixer"}
            </p>
          ) : null}
        </div>
        <button
          aria-label="Close signal path"
          className="-mr-2 flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-zinc-500 hover:bg-white/8 hover:text-white"
          onClick={() => {
            onClose();
            anchor.current?.focus();
          }}
          type="button"
        >
          <X className="h-4 w-4" />
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        {path ? (
          <ol aria-label="Stages" className="relative">
            {path.stages.map((stage, index) => (
              <StageRow
                expanded={selected === stage.id}
                key={stage.id}
                last={index === path.stages.length - 1}
                onToggle={() =>
                  setSelected((current) =>
                    current === stage.id ? null : stage.id,
                  )
                }
                stage={stage}
              />
            ))}
          </ol>
        ) : (
          <p className="px-2 py-6 text-sm text-zinc-500">
            {loading ? "Reading the signal path..." : "Nothing is playing."}
          </p>
        )}
        {path && !path.serverKnown ? (
          <p className="mx-2 mt-1 rounded-lg bg-white/4 px-3 py-2 text-xs leading-5 text-zinc-400">
            The server&apos;s part of the path could not be loaded, so the
            source is described from the library and server processing is not
            shown.
          </p>
        ) : null}
        {path ? (
          <p className="mx-2 mt-3 text-[11px] leading-4 text-zinc-500">
            {path.description} The light describes formats and processing, not
            the recording itself.
          </p>
        ) : null}
      </div>
      <footer className="space-y-3 border-t border-white/6 px-5 py-3">
        {!casting ? (
          <LevelingControl
            mode={levelingMode}
            onChange={onLevelingModeChange}
          />
        ) : null}
        <div className="flex items-center justify-between gap-3 text-xs text-zinc-500">
          <span className="flex items-center gap-1.5">
            <Gauge className="h-3.5 w-3.5" />
            Streaming:{" "}
            <span className="text-zinc-300">
              {casting
                ? "Cast compatible"
                : streamBitrate > 0
                  ? `MP3 ${streamBitrate} kbps`
                  : "Original file"}
            </span>
          </span>
          <span className="flex items-center gap-1">
            {onOpenDsp ? (
              <button
                className="flex items-center gap-1 rounded-full px-2 py-1 text-zinc-300 hover:bg-white/6 hover:text-white"
                onClick={() => {
                  onClose();
                  onOpenDsp();
                }}
                type="button"
              >
                <SlidersHorizontal className="h-3.5 w-3.5" />
                DSP engine
              </button>
            ) : null}
            <Link
              className="flex items-center gap-1 rounded-full px-2 py-1 text-zinc-400 hover:bg-white/6 hover:text-white"
              href="/settings"
              onClick={onClose}
            >
              <Settings2 className="h-3.5 w-3.5" />
              Settings
            </Link>
          </span>
        </div>
        <Legend />
      </footer>
    </div>,
    document.body,
  );
}

function StageRow({
  expanded,
  last,
  onToggle,
  stage,
}: {
  expanded: boolean;
  last: boolean;
  onToggle: () => void;
  stage: SignalStage;
}) {
  const Icon = STAGE_ICONS[stage.kind];
  const color = stage.quality ? signalQualityColors[stage.quality] : "#71717a";
  return (
    <li className="relative" data-stage={stage.id}>
      {!last ? (
        <span
          aria-hidden="true"
          className="absolute -bottom-2 left-6 top-10 w-px bg-white/12"
        />
      ) : null}
      <button
        aria-expanded={expanded}
        className="flex w-full items-start gap-3 rounded-xl px-2 py-2 text-left transition-colors hover:bg-white/4 focus-visible:bg-white/6 focus-visible:outline-none"
        onClick={onToggle}
        type="button"
      >
        <span className="relative z-10 flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/14 bg-[#141416] text-zinc-300">
          <Icon className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1 pt-0.5">
          <span className="flex items-center gap-1.5 text-sm font-medium text-zinc-100">
            <span className="truncate">{stage.title}</span>
            <QualityGlyph quality={stage.quality} size={11} />
            <span className="sr-only">
              {stage.quality ? signalQualityLabels[stage.quality] : "Unknown"}
            </span>
          </span>
          <span
            className="mt-0.5 block text-[13px] leading-5"
            style={{ color: stage.quality ? `${color}` : "#a1a1aa" }}
          >
            {stage.detail}
          </span>
          {expanded ? (
            <span className="mt-1.5 block text-xs leading-5 text-zinc-400">
              {stage.explanation}
            </span>
          ) : null}
        </span>
      </button>
    </li>
  );
}

function LevelingControl({
  mode,
  onChange,
}: {
  mode: VolumeLevelingMode;
  onChange: (mode: VolumeLevelingMode) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-xs font-medium text-zinc-300">Volume leveling</span>
      <div
        aria-label="Volume leveling"
        className="flex rounded-full bg-white/6 p-0.5"
        role="radiogroup"
      >
        {volumeLevelingModes.map((option) => (
          <button
            aria-checked={mode === option}
            className={cn(
              "h-6 rounded-full px-2.5 text-[11px] font-medium transition-colors",
              mode === option
                ? "bg-white text-black"
                : "text-zinc-400 hover:text-white",
            )}
            key={option}
            onClick={() => onChange(option)}
            role="radio"
            type="button"
          >
            {volumeLevelingModeLabels[option]}
          </button>
        ))}
      </div>
    </div>
  );
}

function Legend() {
  return (
    <ul
      aria-label="Light colours"
      className="grid grid-cols-2 gap-x-3 gap-y-1 text-[11px] text-zinc-500"
    >
      {QUALITY_ORDER.map((quality) => (
        <li className="flex items-center gap-1.5" key={quality}>
          <QualityGlyph quality={quality} size={10} />
          {signalQualityLabels[quality]}
        </li>
      ))}
    </ul>
  );
}

/** Roon-style album "Format" line: source quality light and format range. */
export function AlbumFormat({
  audios,
}: {
  audios: readonly (AudioProperties | null | undefined)[];
}) {
  const format = describeCollectionFormat(audios);
  if (!format) return null;
  return (
    <>
      <span className="text-zinc-500">•</span>
      <span
        className="inline-flex items-center gap-1.5 text-zinc-300"
        title={
          format.quality
            ? `Source: ${signalQualityLabels[format.quality]}`
            : undefined
        }
      >
        <QualityGlyph quality={format.quality} size={12} />
        <span>{format.summary}</span>
        {format.highRes ? <HiResBadge /> : null}
      </span>
    </>
  );
}
