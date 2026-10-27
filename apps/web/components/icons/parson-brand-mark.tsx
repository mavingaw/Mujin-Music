import { cn } from "@/lib/cn";

/**
 * The Mujin Music symbol ("Ensō Signal"): an open ring with the signal light
 * in its break. The ring takes the text colour; the dot (`.mj-signal`) takes
 * the live signal-path quality colour (`--mujin-quality`), copper when
 * nothing says otherwise.
 */
export default function ParsonBrandMark({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={cn("text-mujin-text", className)}
      focusable="false"
      viewBox="0 0 512 512"
    >
      <path
        d="M456.91 202.17A208 208 0 1 1 309.83 55.09L293.89 114.59A133.12 133.12 0 1 0 397.41 218.11Z"
        fill="currentColor"
      />
      <circle
        className="mj-signal"
        cx="382.12"
        cy="129.88"
        fill="#d98b5f"
        r="44.72"
      />
    </svg>
  );
}
