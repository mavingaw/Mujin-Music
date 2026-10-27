"use client";

import {
  describeCollectionFormat,
  formatDecibels,
  formatRealtimeFactor,
  SIGNAL_CLIPPING_COLOR,
  signalLightColor,
  signalLightLabel,
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
import { OutputArtwork } from "./output-artwork";
import {
  volumeLevelingModeLabels,
  volumeLevelingModes,
} from "./volume-leveling";

/** Where a network player's format limits came from, in a few words. */
const NETWORK_LIMIT_NOTES: Record<string, string> = {
  declared: "formats declared by the player",
  protocol: "format fixed by AirPlay",
  known_model: "maker's published limits",
  assumed: "rates assumed: the player does not list them",
};

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
  clipping = false,
}: {
  quality: SignalQuality | null;
  size?: number;
  className?: string;
  /** The DSP chain is clipping: the light turns red. */
  clipping?: boolean;
}) {
  const color = clipping
    ? SIGNAL_CLIPPING_COLOR
    : quality
      ? signalQualityColors[quality]
      : "#71717a";
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
  return signalLightLabel(path);
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
  /** The zone the path belongs to when the server plays it. */
  zoneId?: string | null;
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
        data-signal-clipping={path?.clipping ? "true" : undefined}
        data-signal-quality={path?.quality ?? "unknown"}
        onClick={() => setOpen((value) => !value)}
        ref={triggerRef}
        title={variant === "compact" ? label : undefined}
        type="button"
      >
        <QualityGlyph
          clipping={path?.clipping}
          quality={path?.quality ?? null}
          size={variant === "compact" ? 15 : 13}
        />
        {variant === "pill" && path ? (
          <>
            <span
              className="shrink-0 font-semibold"
              style={{ color: signalLightColor(path) }}
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

/** Wide enough that stage details stay on one line. */
const PANEL_WIDTH = 500;
/**
 * On a desktop screen the panel gains a side column for the explanation,
 * so the whole chain stays one column tall and nothing scrolls at 1440x900.
 */
const WIDE_PANEL_WIDTH = 780;
const WIDE_VIEWPORT = 1100;

/** The panel has room for the explanation column. */
function useWidePanel() {
  const [wide, setWide] = useState(false);
  useLayoutEffect(() => {
    const update = () => setWide(window.innerWidth >= WIDE_VIEWPORT);
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  return wide;
}

function usePanelPosition(
  anchor: React.RefObject<HTMLElement | null>,
  panel: React.RefObject<HTMLElement | null>,
  wide: boolean,
) {
  const [style, setStyle] = useState<React.CSSProperties>({
    visibility: "hidden",
  });
  const place = useCallback(() => {
    const trigger = anchor.current?.getBoundingClientRect();
    const element = panel.current;
    if (!trigger || !element) return;
    const margin = 12;
    const width = Math.min(
      wide ? WIDE_PANEL_WIDTH : PANEL_WIDTH,
      window.innerWidth - margin * 2,
    );
    // The whole viewport is available: a long chain (decode, the DSP
    // stages, transport, output, the driver's confirmation) fits at
    // 1440x900 instead of scrolling inside a short box.
    const maxHeight = window.innerHeight - margin * 2;
    const height = Math.min(element.offsetHeight, maxHeight);
    const left = Math.min(
      Math.max(margin, trigger.left + trigger.width / 2 - width / 2),
      window.innerWidth - width - margin,
    );
    const above = trigger.top - margin - height;
    const below = trigger.bottom + margin;
    const top =
      above >= margin
        ? above
        : below + height <= window.innerHeight - margin
          ? below
          : // Neither side has room: cover the trigger rather than clip.
            window.innerHeight - height - margin;
    setStyle({ left, maxHeight, top: Math.max(margin, top), width });
  }, [anchor, panel, wide]);
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
  zoneId = null,
}: PopoverProps & {
  anchor: React.RefObject<HTMLButtonElement | null>;
  id: string;
  onClose: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const wide = useWidePanel();
  const style = usePanelPosition(anchor, panelRef, wide);
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

  // When the chain is taller than the screen, keep where it ends (the
  // device and the driver's confirmation) in view rather than the source.
  const lastStage = path?.stages.at(-1)?.id;
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list || !lastStage || list.scrollHeight <= list.clientHeight) return;
    list
      .querySelector(`[data-stage="${lastStage}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [lastStage]);

  if (typeof document === "undefined") return null;
  const quality = path?.quality ?? null;
  const serverOutput =
    path?.stages.find((stage) => stage.id === "native-output") ?? null;
  const device = path?.outputDevice ?? null;
  const selectedStage =
    path?.stages.find((stage) => stage.id === selected) ?? null;
  const network = device?.network ?? null;
  const deviceFacts = device
    ? network
      ? [
          [network.manufacturer, network.model]
            .filter(Boolean)
            .join(" ")
            .replace(/^(\S+) \1 /, "$1 ") || null,
          `${network.protocol_label} at ${network.endpoint}`,
          NETWORK_LIMIT_NOTES[network.caps_source],
          `from ${device.host}`,
        ].filter(Boolean)
      : [
          device.usb_id ? `USB ${device.usb_id}` : null,
          device.usb_speed,
          device.transport,
          `on ${device.host}`,
        ].filter(Boolean)
    : [];
  const reporter = path?.hardwareReporter ?? "driver";

  return createPortal(
    <div
      aria-labelledby={titleId}
      className="fixed z-120 flex flex-col overflow-hidden rounded-2xl border border-white/10 bg-[#0d0d0f]/98 text-left text-white shadow-[0_24px_80px_rgba(0,0,0,0.75)] outline-none backdrop-blur-xl"
      data-signal-path-panel
      id={id}
      onContextMenu={(event) => event.stopPropagation()}
      ref={panelRef}
      role="dialog"
      style={style}
      tabIndex={-1}
    >
      <header className="flex items-start gap-3 border-b border-white/6 px-5 pb-3 pt-4">
        {serverOutput && network ? (
          <OutputArtwork
            className="mt-1"
            zoneId={zoneId}
            hasIcon={network.has_icon}
            kind={network.kind}
            protocol={network.protocol}
            size={48}
          />
        ) : null}
        <div className="min-w-0 flex-1">
          <h2 className="text-[17px] font-semibold leading-6" id={titleId}>
            Signal path:{" "}
            <span
              style={{
                color: path
                  ? signalLightColor(path)
                  : quality
                    ? signalQualityColors[quality]
                    : undefined,
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
          {path?.bitPerfect && serverOutput ? (
            <p
              className="mt-1.5 flex items-center gap-1.5 text-xs font-medium"
              data-bit-perfect
              style={{ color: signalQualityColors.lossless }}
            >
              <QualityGlyph quality="lossless" size={10} />
              Bit-perfect to {serverOutput.title}
            </p>
          ) : path?.clipping && serverOutput ? (
            <p
              className="mt-1.5 flex items-center gap-1.5 text-xs font-medium"
              data-clipping-to-output
              style={{ color: SIGNAL_CLIPPING_COLOR }}
            >
              <QualityGlyph clipping quality="enhanced" size={10} />
              Processed on the server and sent to {serverOutput.title}, but
              clipping changed some samples
            </p>
          ) : path?.exactAfterProcessing && serverOutput ? (
            <p
              className="mt-1.5 flex items-center gap-1.5 text-xs font-medium"
              data-exact-after-processing
              style={{ color: signalQualityColors.enhanced }}
            >
              <QualityGlyph quality="enhanced" size={10} />
              Processed on the server, then bit-exact to {serverOutput.title}
            </p>
          ) : path?.bitPerfectToOutput && path.quality !== "lossless" ? (
            <p
              className="mt-1.5 flex items-center gap-1.5 text-xs font-medium"
              style={{ color: signalQualityColors.lossless }}
            >
              <QualityGlyph quality="lossless" size={10} />
              Bit-perfect up to the {casting ? "Cast device" : "system mixer"}
            </p>
          ) : null}
          {serverOutput && deviceFacts.length ? (
            <p
              className="mt-0.5 text-[11px] leading-4 text-zinc-400"
              data-output-device
            >
              {deviceFacts.join(" · ")}
            </p>
          ) : null}
          {serverOutput && device?.kernel_emulator ? (
            <p
              className="mt-0.5 text-[11px] leading-4 text-amber-300/90"
              data-kernel-emulator
            >
              Test bench: Mujin Music&apos;s kernel-ABI emulator answers for the
              kernel here, so this proves the hw driver path, not a DAC.
            </p>
          ) : null}
          {serverOutput && network ? (
            <p
              className="mt-0.5 text-[11px] leading-4 text-amber-300/90"
              data-network-verification
            >
              {network.protocol === "air_play"
                ? "Tested against Mujin Music's protocol-level AirPlay receiver, not yet a real AirPlay device."
                : "Tested against Mujin Music's protocol-level UPnP/OpenHome renderer, not yet a real streamer."}
            </p>
          ) : null}
          {path?.hardwareVerified !== null &&
          path?.hardwareVerified !== undefined ? (
            <p
              className="mt-1 text-[11px] leading-4"
              data-hardware-verified={path.hardwareVerified}
              style={{
                color: path.hardwareVerified
                  ? path.quality === "lossless"
                    ? signalQualityColors.lossless
                    : "#d4d4d8"
                  : signalQualityColors.high_quality,
              }}
            >
              {path.hardwareVerified
                ? `Confirmed by the ${reporter}: ${path.hardwareReport}, exactly the format Mujin Music ${reporter === "renderer" ? "sends" : "opened"}.${
                    path.quality === "low_quality"
                      ? " The light is yellow because the file itself is lossy."
                      : ""
                  }`
                : `${reporter === "renderer" ? "Renderer" : "Driver"} report: ${path.hardwareReport}, not the format Mujin Music ${reporter === "renderer" ? "sends" : "opened"}: something below Mujin Music converts the stream.`}
            </p>
          ) : null}
          {path && !serverOutput && !casting ? (
            <p className="mt-1 text-[11px] leading-4 text-zinc-500">
              Browsers only reach the device in shared mode. For exclusive,
              bit-perfect playback, pick one of the server&apos;s zones in the
              zone picker.
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
      <div
        className={cn("flex min-h-0 flex-1", wide ? "flex-row" : "flex-col")}
      >
        <div
          className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
          data-signal-stages
          ref={listRef}
        >
          {path ? (
            <ol aria-label="Stages" className="relative">
              {path.stages.map((stage, index) => (
                <StageRow
                  expanded={selected === stage.id}
                  inline={!wide}
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
          {!wide ? <PathNotes path={path} /> : null}
        </div>
        {wide && path ? (
          <aside
            aria-label="About this path"
            className="w-[290px] shrink-0 space-y-2 overflow-y-auto border-l border-white/6 px-3 py-3"
            data-signal-aside
          >
            {selectedStage ? (
              <div
                className="rounded-lg bg-white/4 px-3 py-2 text-xs leading-5"
                data-stage-explanation={selectedStage.id}
              >
                <p className="font-medium text-zinc-100">
                  {selectedStage.title}
                </p>
                <p className="mt-0.5 text-zinc-400">
                  {selectedStage.explanation}
                </p>
              </div>
            ) : null}
            {path.processing ? <ProcessingSummary path={path} /> : null}
            <PathNotes path={path} />
          </aside>
        ) : null}
      </div>
      {!wide && path?.processing ? (
        // Pinned below the list, so clipping is visible however long the
        // chain is.
        <div className="shrink-0 px-3 pb-2">
          <ProcessingSummary path={path} />
        </div>
      ) : null}
      <footer className="shrink-0 space-y-2 border-t border-white/6 px-5 py-2.5">
        {!casting && !serverOutput ? (
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
                : serverOutput
                  ? "Played by the server"
                  : path?.stages.some((stage) => stage.id === "relay")
                    ? "Live stream, relayed by Mujin Music"
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
              href={serverOutput ? "/settings?tab=outputs" : "/settings"}
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

/** The sentence under the light, and what could not be loaded. */
function PathNotes({ path }: { path: SignalPath | null }) {
  if (!path) return null;
  return (
    <>
      {!path.serverKnown ? (
        <p className="mx-2 mt-1 rounded-lg bg-white/4 px-3 py-2 text-xs leading-5 text-zinc-400">
          The server&apos;s part of the path could not be loaded, so the source
          is described from the library and server processing is not shown.
        </p>
      ) : null}
      <p
        className="mx-2 mt-2 text-[11px] leading-4 text-zinc-500"
        data-signal-description
      >
        {path.description} The light describes formats and processing, not the
        recording itself.
      </p>
    </>
  );
}

/** Roon's MUSE footer: where the chain runs and how fast. */
function ProcessingSummary({ path }: { path: SignalPath }) {
  const processing = path.processing;
  if (!processing) return null;
  const status = processing.status;
  return (
    <div
      className="mx-2 space-y-0.5 rounded-lg border px-3 py-1.5 text-xs leading-5"
      data-signal-processing
      style={{
        borderColor: status?.clipping
          ? `${SIGNAL_CLIPPING_COLOR}66`
          : "rgba(255,255,255,0.08)",
      }}
    >
      <p className="flex items-center gap-1.5 text-zinc-300">
        <Cpu className="h-3.5 w-3.5 text-zinc-500" />
        {processing.engine}
      </p>
      <p className="flex flex-wrap gap-x-3 text-zinc-500">
        <span>
          Processing speed{" "}
          <span className="font-semibold tabular-nums text-zinc-200">
            {status?.realtime_factor
              ? formatRealtimeFactor(status.realtime_factor)
              : "measured while playing"}
          </span>
        </span>
        {status?.peak_dbfs !== null && status?.peak_dbfs !== undefined ? (
          <span className="tabular-nums">
            Peak {formatDecibels(status.peak_dbfs)}FS
          </span>
        ) : null}
      </p>
      {status?.clipping ? (
        <p
          className="font-semibold"
          data-signal-clipping-note
          style={{ color: SIGNAL_CLIPPING_COLOR }}
        >
          Clipping detected: {status.clipped_samples.toLocaleString()} samples
          were limited. Add headroom in the DSP engine.
        </p>
      ) : null}
      {processing.notes.map((note) => (
        <p className="text-amber-300/80" key={note}>
          {note}
        </p>
      ))}
    </div>
  );
}

function StageRow({
  expanded,
  inline,
  last,
  onToggle,
  stage,
}: {
  expanded: boolean;
  /** The explanation opens under the row (else in the side column). */
  inline: boolean;
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
          className="absolute -bottom-1 left-5.5 top-8 w-px bg-white/12"
        />
      ) : null}
      <button
        aria-expanded={expanded}
        className={cn(
          "flex w-full items-start gap-3 rounded-xl px-2 py-1 text-left transition-colors hover:bg-white/4 focus-visible:bg-white/6 focus-visible:outline-none",
          expanded && !inline && "bg-white/6",
        )}
        onClick={onToggle}
        type="button"
      >
        <span className="relative z-10 flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-white/14 bg-[#141416] text-zinc-300">
          <Icon className="h-3.5 w-3.5" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5 text-sm font-medium text-zinc-100">
            <span className="truncate">{stage.title}</span>
            <QualityGlyph quality={stage.quality} size={11} />
            <span className="sr-only">
              {stage.quality ? signalQualityLabels[stage.quality] : "Unknown"}
            </span>
          </span>
          <span
            className="block text-[13px] leading-5"
            style={{ color: stage.quality ? `${color}` : "#a1a1aa" }}
          >
            {stage.detail}
          </span>
          {expanded && inline ? (
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
      className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-zinc-500"
    >
      {QUALITY_ORDER.map((quality) => (
        <li className="flex items-center gap-1.5" key={quality}>
          <QualityGlyph quality={quality} size={10} />
          {signalQualityLabels[quality]}
        </li>
      ))}
      <li className="flex items-center gap-1.5">
        <QualityGlyph clipping quality="enhanced" size={10} />
        Clipping
      </li>
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
