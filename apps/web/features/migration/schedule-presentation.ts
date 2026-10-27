import type { BackupScheduleStatus } from "@parson/music-sdk";

export type ScheduleSummary = { tone: "good" | "warn" | "off"; text: string };

function when(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString("en-GB", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** One line under the backup folder: is the off-machine copy working? */
export function scheduleSummary(status: BackupScheduleStatus): ScheduleSummary {
  const report = status.destination_report;
  if (status.frequency === "off" && !status.destination) {
    return {
      tone: "warn",
      text: "Automatic backups are off. Back up by hand before moving to a new server.",
    };
  }
  if (!status.destination) {
    return {
      tone: "off",
      text: `Backups stay on this machine, in ${status.local_directory}.`,
    };
  }
  const failedLast =
    report.last_error_at &&
    (!report.last_copied_at || report.last_error_at > report.last_copied_at);
  if (failedLast) {
    return {
      tone: "warn",
      text: `The last copy failed (${when(report.last_error_at!)}): ${report.last_error}`,
    };
  }
  if (report.last_copied_at) {
    return {
      tone: "good",
      text: `Last copied ${when(report.last_copied_at)}. Keeping the newest ${status.destination_keep} there.`,
    };
  }
  return {
    tone: "off",
    text: `The next backup is copied there${status.frequency === "off" ? " when you back up by hand" : ""}. Keeping the newest ${status.destination_keep}.`,
  };
}
