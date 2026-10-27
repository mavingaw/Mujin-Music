import type {
  HistoryBucket,
  HistoryBucketCount,
  HistoryPeriod,
} from "@parson/music-sdk";

const pad = (value: number) => String(value).padStart(2, "0");

function dayKey(date: Date) {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function monthKey(date: Date) {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}`;
}

/**
 * Every bucket of a bounded period, zero-filled, so quiet days show as
 * gaps in the chart instead of disappearing. Unbounded periods keep the
 * buckets that have plays.
 */
export function fillBuckets(
  timeline: HistoryBucketCount[],
  period: HistoryPeriod,
  bucket: HistoryBucket,
): HistoryBucketCount[] {
  if (!period.from || !period.to || (bucket !== "day" && bucket !== "month")) {
    return timeline;
  }
  const byKey = new Map(timeline.map((item) => [item.bucket, item]));
  const filled: HistoryBucketCount[] = [];
  const end = new Date(period.to);
  const cursor = new Date(period.from);
  for (let guard = 0; cursor < end && guard < 400; guard += 1) {
    const key = bucket === "day" ? dayKey(cursor) : monthKey(cursor);
    filled.push(
      byKey.get(key) ?? { bucket: key, plays: 0, listenedSeconds: 0 },
    );
    if (bucket === "day") cursor.setDate(cursor.getDate() + 1);
    else cursor.setMonth(cursor.getMonth() + 1, 1);
  }
  return filled;
}

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/** Short axis label for a bucket key: "12", "Mar", "W 14 Oct", "2025". */
export function bucketLabel(key: string, bucket: HistoryBucket): string {
  const [year = 1970, month, day] = key.split("-").map(Number);
  if (bucket === "year") return String(year);
  if (bucket === "month")
    return `${MONTHS[(month ?? 1) - 1]} ${String(year).slice(2)}`;
  if (bucket === "week") return `${day} ${MONTHS[(month ?? 1) - 1]}`;
  return String(day);
}

/** Long label for a tooltip: "Tue 14 Oct 2025", "Week of 13 Oct", "March 2025". */
export function bucketTitle(key: string, bucket: HistoryBucket): string {
  const [year = 1970, month, day] = key.split("-").map(Number);
  if (bucket === "year") return String(year);
  const date = new Date(year, (month ?? 1) - 1, day ?? 1);
  if (bucket === "month") {
    return date.toLocaleDateString("en-GB", { month: "long", year: "numeric" });
  }
  const text = date.toLocaleDateString("en-GB", {
    weekday: bucket === "day" ? "short" : undefined,
    day: "numeric",
    month: "short",
    year: "numeric",
  });
  return bucket === "week" ? `Week of ${text}` : text;
}

/** "+12%", "−8%", or null when there is nothing to compare with. */
export function formatChange(change: number | null): string | null {
  if (change === null || !Number.isFinite(change)) return null;
  const percent = Math.round(change * 100);
  if (percent === 0) return "±0%";
  return `${percent > 0 ? "+" : "−"}${Math.abs(percent)}%`;
}

/** Hours of day reordered from 6am so a night of listening stays together. */
export function clockOrder(hours: number[]): { hour: number; plays: number }[] {
  return Array.from({ length: 24 }, (_, index) => {
    const hour = (index + 6) % 24;
    return { hour, plays: hours[hour] ?? 0 };
  });
}

/** Ticks to label on an axis of `count` bars without crowding. */
export function labelEvery(count: number, room = 12): number {
  return Math.max(1, Math.ceil(count / room));
}
