import {
  STREAMING_PROVIDER_NAMES,
  type StreamingProviderId,
} from "@parson/music-sdk";
import { HardDrive } from "lucide-react";
import { cn } from "@/lib/cn";
import type { FormatTone } from "./streaming-model";

/**
 * A neutral monogram for each service. Parson does not reproduce the
 * services' logos; the name next to it identifies the source.
 */
export function ProviderMark({
  provider,
  className,
  onLight = false,
}: {
  provider: StreamingProviderId;
  className?: string;
  /** Use on white surfaces, where TIDAL's white mark would vanish. */
  onLight?: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-[5px] text-[0.5625rem] font-black leading-none",
        provider === "tidal"
          ? onLight
            ? "bg-black text-white"
            : "bg-white text-black"
          : "bg-[#3e8ef7] text-white",
        className,
      )}
    >
      {provider === "tidal" ? "T" : "Q"}
    </span>
  );
}

/** Marks where an album or track plays from. */
export function SourceBadge({
  provider,
  variant = "inline",
  className,
}: {
  provider: StreamingProviderId;
  variant?: "inline" | "overlay";
  className?: string;
}) {
  const name = STREAMING_PROVIDER_NAMES[provider];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full text-[0.6875rem] font-semibold leading-none",
        variant === "overlay"
          ? "bg-black/70 py-1 pl-1 pr-2 text-white shadow-[0_1px_8px_rgba(0,0,0,0.4)] backdrop-blur-md"
          : "text-zinc-400",
        className,
      )}
      title={`Streams from ${name}`}
    >
      <ProviderMark provider={provider} />
      <span>{name}</span>
      {variant === "overlay" && <span className="sr-only">album</span>}
    </span>
  );
}

export function LocalSourceBadge({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 text-[0.6875rem] font-semibold text-zinc-400",
        className,
      )}
      title="Plays from your music folder"
    >
      <HardDrive aria-hidden="true" className="h-3.5 w-3.5" />
      Local
    </span>
  );
}

const toneClasses: Record<FormatTone, string> = {
  "hi-res": "border-amber-300/30 bg-amber-300/10 text-amber-200",
  lossless: "border-mujin-copper/25 bg-mujin-copper/8 text-mujin-copper-hover",
  lossy: "border-white/10 bg-white/5 text-zinc-400",
};

/** Quality badge such as `24/192 FLAC`. */
export function FormatBadge({
  label,
  tone,
  className,
}: {
  label: string;
  tone: FormatTone | null;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-[5px] border px-1.5 py-0.5 text-[0.6562rem] font-semibold tabular-nums leading-none tracking-wide",
        toneClasses[tone ?? "lossy"],
        className,
      )}
      title={
        tone === "hi-res"
          ? "High resolution, lossless"
          : tone === "lossless"
            ? "Lossless, CD quality"
            : "Lossy"
      }
    >
      {label}
    </span>
  );
}
