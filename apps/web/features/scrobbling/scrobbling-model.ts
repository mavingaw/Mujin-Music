import type { ScrobbleImportStatus, ScrobbleLogEntry } from "@parson/music-sdk";

export type Tone = "ok" | "warn" | "error" | "idle";

const count = new Intl.NumberFormat("en");

/** The status chip of one listen in the activity list. */
export function activityLabel(entry: ScrobbleLogEntry): {
  tone: Tone;
  text: string;
} {
  switch (entry.status) {
    case "sent":
      return { tone: "ok", text: "Scrobbled" };
    case "pending":
      return {
        tone: "warn",
        text: entry.attempts > 0 ? "Waiting to retry" : "Sending",
      };
    case "ignored":
      return { tone: "idle", text: "Not counted" };
    default:
      return { tone: "error", text: "Failed" };
  }
}

export function importProgressLabel(status: ScrobbleImportStatus): string {
  const read = count.format(status.fetched);
  const added = count.format(status.imported);
  if (status.status === "running") {
    return status.fetched
      ? `Importing… ${read} listens read, ${added} added`
      : "Importing…";
  }
  if (status.status === "failed") {
    return `Import stopped: ${status.error ?? "unknown error"}`;
  }
  const unmatched = status.fetched - status.matched;
  const already = status.matched - status.imported;
  const parts = [`${added} listens added`];
  if (already > 0) parts.push(`${count.format(already)} already here`);
  if (unmatched > 0)
    parts.push(`${count.format(unmatched)} not in your library`);
  return `Imported ${read} listens: ${parts.join(", ")}`;
}

/** "14:05", "Yesterday 22:10" or "3 Mar" for a listen. */
export function listenTime(
  unixSeconds: number,
  now: Date = new Date(),
): string {
  const date = new Date(unixSeconds * 1000);
  const time = date.toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  const day = 24 * 60 * 60 * 1000;
  if (date.getTime() >= startOfToday.getTime()) return time;
  if (date.getTime() >= startOfToday.getTime() - day)
    return `Yesterday ${time}`;
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}
