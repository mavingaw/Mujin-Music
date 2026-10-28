import {
  SIGNAL_CLIPPING_COLOR as BRAND_CLIPPING,
  signalLightColor as brandLightColor,
  signalQualityColors as brandColors,
  type SignalPath,
  type SignalQuality,
} from "@parson/music-sdk";

/**
 * The signal-path light's colours as theme variables (with the brand hex as
 * the fallback), so a light theme draws them in tones that keep their
 * contrast on paper, and High Contrast in its own.
 */
const VARIABLES: Record<SignalQuality, string> = {
  lossless: "--color-quality-lossless",
  enhanced: "--color-quality-enhanced",
  high_quality: "--color-quality-high",
  low_quality: "--color-quality-low",
};

export const signalQualityColors = Object.fromEntries(
  (Object.keys(VARIABLES) as SignalQuality[]).map((quality) => [
    quality,
    `var(${VARIABLES[quality]}, ${brandColors[quality]})`,
  ]),
) as Record<SignalQuality, string>;

export const SIGNAL_CLIPPING_COLOR = `var(--color-quality-clipping, ${BRAND_CLIPPING})`;

/** The light's colour for a path, as a theme variable. */
export function signalLightColor(
  path: Pick<SignalPath, "quality" | "clipping">,
): string {
  const brand = brandLightColor(path);
  if (brand === BRAND_CLIPPING) return SIGNAL_CLIPPING_COLOR;
  const quality = (Object.keys(brandColors) as SignalQuality[]).find(
    (key) => brandColors[key] === brand,
  );
  return quality ? signalQualityColors[quality] : brand;
}
