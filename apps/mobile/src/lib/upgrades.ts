import {
  enhancedStatusLabel,
  isUpgradeOpen,
  upgradeStatusLabel,
  type AlbumUpgradeStatus,
} from "@parson/music-sdk";

export interface UpgradeChip {
  key: string;
  label: string;
  color: string;
  /** Something is still moving. */
  busy: boolean;
}

/** The palette colours the chips are drawn in (the active theme's). */
export type ChipPalette = {
  accentSoft: string;
  secondary: string;
  enhanced: string;
};

/**
 * The album page's Better versions line on the phone: a lossless copy on
 * its way ("Lossless version on its way · 45%", "Handed to Lidarr") and
 * AI-enhanced copies, which are never called lossless.
 */
export function upgradeChips(
  status: AlbumUpgradeStatus | undefined,
  palette: ChipPalette,
) {
  const chips: UpgradeChip[] = [];
  if (!status) return chips;
  const request = status.request;
  if (
    request &&
    request.state !== "cancelled" &&
    request.state !== "not_found"
  ) {
    chips.push({
      key: "request",
      label: upgradeStatusLabel(request),
      color:
        request.state === "fake_lossless" || request.state === "failed"
          ? palette.secondary
          : palette.accentSoft,
      busy: isUpgradeOpen(request.state) && request.state !== "handed_off",
    });
  }
  const enhanced = enhancedStatusLabel(status);
  if (enhanced) {
    chips.push({
      key: "enhanced",
      label: enhanced,
      color: palette.enhanced,
      busy: false,
    });
  }
  if (status.enhance_queued > 0) {
    chips.push({
      key: "queued",
      label: `${status.enhance_queued} track(s) being enhanced`,
      color: palette.enhanced,
      busy: true,
    });
  }
  return chips;
}
