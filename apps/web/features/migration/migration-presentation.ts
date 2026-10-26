import type { FolderBackup } from "@parson/music-sdk";

export function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const unit = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  return `${(bytes / 1024 ** unit).toFixed(unit ? 1 : 0)} ${units[unit]}`;
}

function formatDate(value: string | null) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** "Home Library · 20 Oct 2026, 08:00" */
export function backupTitle(backup: FolderBackup) {
  const when = formatDate(backup.created_at);
  const name = backup.library_name ?? "Parson library";
  return when ? `${name} · ${when}` : name;
}

/** "1.2 MB · Parson 1.0.1 · from /home/ana/Music" */
export function backupDetail(backup: FolderBackup) {
  return [
    formatBytes(backup.size_bytes),
    backup.product_version ? `Parson ${backup.product_version}` : "",
    backup.library_roots.length
      ? `music was in ${backup.library_roots.join(", ")}`
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * Waits for the server to go away and come back after a restart. Resolves
 * true once `/health/ready` answers again with a different boot id, false
 * after `timeoutMs`.
 */
export async function waitForRestart({
  previousBootId,
  readBootId,
  timeoutMs = 120_000,
  intervalMs = 1_000,
  sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms)),
  now = () => Date.now(),
}: {
  previousBootId: string | null;
  readBootId: () => Promise<string | null>;
  timeoutMs?: number;
  intervalMs?: number;
  sleep?: (ms: number) => Promise<unknown>;
  now?: () => number;
}): Promise<boolean> {
  const deadline = now() + timeoutMs;
  let wentAway = false;
  while (now() < deadline) {
    let bootId: string | null = null;
    try {
      bootId = await readBootId();
    } catch {
      wentAway = true;
    }
    if (bootId && (bootId !== previousBootId || (wentAway && !previousBootId)))
      return true;
    await sleep(intervalMs);
  }
  return false;
}

/** Reads the running server's boot id from its public manifest. */
export async function readServerBootId(
  fetcher: typeof fetch = fetch,
): Promise<string | null> {
  const response = await fetcher("/.well-known/parson", { cache: "no-store" });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const manifest = (await response.json()) as { bootId?: unknown };
  return typeof manifest.bootId === "string" ? manifest.bootId : null;
}
