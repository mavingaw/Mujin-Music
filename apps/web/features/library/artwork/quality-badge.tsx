import { cn } from "@/lib/cn";

/**
 * A small format badge ("24/96", "16/44.1", "MP3") in the mono readout face.
 * High-resolution lossless gets the copper signal dot; plain lossless a
 * quiet dot; lossy none. Always text, never colour alone.
 */
export function QualityBadge({
  className,
  hiRes = false,
  label,
  lossless = false,
  variant = "overlay",
}: {
  className?: string;
  hiRes?: boolean;
  label: string;
  lossless?: boolean;
  variant?: "overlay" | "inline";
}) {
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center gap-1.5 rounded-[4px] px-1.5 font-mono text-[10px] font-medium leading-none tracking-[0.02em]",
        variant === "overlay"
          ? "bg-black/62 text-mujin-text shadow-[inset_0_0_0_1px_rgba(242,238,230,0.1)] backdrop-blur-md"
          : "bg-white/[0.05] text-mujin-text-secondary shadow-[inset_0_0_0_1px_rgba(242,238,230,0.1)]",
        className,
      )}
      title={
        hiRes
          ? `High resolution lossless · ${label}`
          : lossless
            ? `Lossless · ${label}`
            : label
      }
    >
      {(hiRes || lossless) && (
        <span
          aria-hidden="true"
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            hiRes ? "bg-quality-lossless" : "bg-mujin-text-tertiary",
          )}
        />
      )}
      {hiRes && <span className="text-mujin-copper">HI-RES</span>}
      {label}
    </span>
  );
}
