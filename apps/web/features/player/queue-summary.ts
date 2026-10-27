/** "12 tracks · 48 min left" for the queue header. */
export function queueSummary(
  queue: { song: { duration?: number } }[],
  currentIndex = 0,
): string {
  if (!queue.length) return "Nothing queued";
  const start = Math.max(0, currentIndex);
  const remaining = queue
    .slice(start)
    .reduce((total, item) => total + (item.song.duration || 0), 0);
  const tracks = `${queue.length} ${queue.length === 1 ? "track" : "tracks"}`;
  const minutes = Math.round(remaining / 60);
  if (minutes <= 0) return tracks;
  const time =
    minutes >= 60
      ? `${Math.floor(minutes / 60)} h ${minutes % 60} min`
      : `${minutes} min`;
  return `${tracks} · ${time} left`;
}

/** A queue row's length: "3:07", "1:02:45", or "" when unknown. */
export function queueRowDuration(seconds: number | undefined): string {
  const total = Math.round(Number(seconds));
  if (!Number.isFinite(total) || total <= 0) return "";
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = String(total % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
}
