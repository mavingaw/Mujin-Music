import { cn } from "@/lib/cn";

/**
 * The Mujin Music horizontal lockup (symbol + hand-drawn wordmark), inlined
 * so the signal dot (`.mj-signal`) follows the live quality colour. "MUJIN"
 * takes the text colour, "MUSIC" the tertiary text colour. Never retype the
 * wordmark in a font.
 */
export default function MujinLockup({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={cn("text-[#f2eee6]", className)}
      focusable="false"
      viewBox="0 0 2275 416"
    >
      <defs>
        {/* Flat tops and bottoms on the monoline capitals, as drawn. */}
        <clipPath id="mj-lockup-mujin">
          <rect height="100" width="485" x="-10" y="0" />
        </clipPath>
        <clipPath id="mj-lockup-music">
          <rect height="100" width="497" x="-10" y="0" />
        </clipPath>
      </defs>
      <path
        d="M408.91 154.17A208 208 0 1 1 261.83 7.09L245.89 66.59A133.12 133.12 0 1 0 349.41 170.11Z"
        fill="currentColor"
      />
      <circle
        className="mj-signal"
        cx="334.12"
        cy="81.88"
        fill="#d98b5f"
        r="44.72"
      />
      <g
        clipPath="url(#mj-lockup-mujin)"
        fill="none"
        stroke="currentColor"
        strokeLinecap="butt"
        strokeLinejoin="miter"
        strokeMiterlimit={12}
        strokeWidth={12}
        transform="translate(566 126) scale(1.64)"
      >
        <path d="M6 100V0L54 70L102 0V100" />
        <path d="M6 0V57A37 37 0 0 0 80 57V0" transform="translate(135 0)" />
        <path d="M56 0V69A25 25 0 0 1 6 69" transform="translate(248 0)" />
        <path d="M6 0V100" transform="translate(337 0)" />
        <path d="M6 100V0L80 100V0" transform="translate(379 0)" />
      </g>
      <g
        clipPath="url(#mj-lockup-music)"
        fill="none"
        stroke="#8e8a84"
        strokeLinecap="butt"
        strokeLinejoin="miter"
        strokeMiterlimit={12}
        strokeWidth={12}
        transform="translate(1492.6 126) scale(1.64)"
      >
        <path d="M6 100V0L54 70L102 0V100" />
        <path d="M6 0V57A37 37 0 0 0 80 57V0" transform="translate(135 0)" />
        <path
          d="M54.6 13.95A22.33 20.68 0 1 0 37 47.36A25.19 23.32 0 1 1 17.15 85.04"
          transform="translate(245 0)"
        />
        <path d="M6 0V100" transform="translate(346 0)" />
        <path
          d="M82.7 20.56A44 44 0 1 0 82.7 79.44"
          transform="translate(385 0)"
        />
      </g>
    </svg>
  );
}
