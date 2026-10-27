import { palette } from "@/constants/colors";
import {
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

/**
 * The album page's Better versions line on the phone: a lossless copy on
 * its way ("Lossless version on its way · 45%", "Handed to Lidarr") and
 * AI-enhanced copies, which are never called lossless.
 */
export function upgradeChips(status: AlbumUpgradeStatus | undefined) {
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
          ? palette.danger
          : palette.accentSoft,
      busy: isUpgradeOpen(request.state) && request.state !== "handed_off",
    });
  }
  if (status.enhanced_copies > 0) {
    chips.push({
      key: "enhanced",
      label:
        status.enhanced_tracks > 0
          ? `AI-enhanced · ${status.enhanced_tracks} of ${status.enhanced_copies} tracks`
          : "AI-enhanced copies available",
      color: palette.qualityEnhanced,
      busy: false,
    });
  }
  if (status.enhance_queued > 0) {
    chips.push({
      key: "queued",
      label: `${status.enhance_queued} track(s) being enhanced`,
      color: palette.qualityEnhanced,
      busy: true,
    });
  }
  return chips;
}
