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

/** "20 Oct 2026, 08:00:05": to the second, so backups made a minute apart
 * (or in the same minute) can be told apart. */
export function formatBackupTime(value: string | null) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

/** "Home Library · 20 Oct 2026, 08:00:05" */
export function backupTitle(backup: FolderBackup) {
  const when = formatBackupTime(backup.created_at);
  const name = backup.library_name ?? "Mujin Music library";
  return when ? `${name} · ${when}` : name;
}

/** "1.2 MB · Parson 1.0.1 · from /home/ana/Music" */
export function backupDetail(backup: FolderBackup) {
  return [
    formatBytes(backup.size_bytes),
    backup.product_version ? `Mujin Music ${backup.product_version}` : "",
    backup.library_roots.length
      ? `music was in ${backup.library_roots.join(", ")}`
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * Titles and details for a list of backups, each one distinguishable: the
 * time is shown to the second with the size; backups that would still read
 * the same (made in the same second) get a sequence number, oldest first,
 * and their exact size in bytes.
 */
export function describeBackups(backups: FolderBackup[]) {
  const titles = backups.map(backupTitle);
  const sizes = backups.map((backup) => formatBytes(backup.size_bytes));
  const sameTitle = (index: number) =>
    titles.filter((title) => title === titles[index]).length > 1;
  return backups.map((backup, index) => {
    let title = backupTitle(backup);
    let size = formatBytes(backup.size_bytes);
    if (sameTitle(index)) {
      const siblings = backups
        .map((other, otherIndex) => ({ other, otherIndex }))
        .filter(({ otherIndex }) => titles[otherIndex] === titles[index])
        .sort(
          (left, right) =>
            left.other.filename.localeCompare(right.other.filename) ||
            left.otherIndex - right.otherIndex,
        );
      const position =
        siblings.findIndex(({ otherIndex }) => otherIndex === index) + 1;
      title = `${title} · #${position} of ${siblings.length}`;
      if (
        siblings.some(
          ({ otherIndex }) =>
            otherIndex !== index && sizes[otherIndex] === sizes[index],
        )
      ) {
        size = `${backup.size_bytes.toLocaleString("en-GB")} bytes`;
      }
    }
    const detail = backupDetail(backup).replace(
      formatBytes(backup.size_bytes),
      size,
    );
    return { title, detail };
  });
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

/**
 * The "also restore away-from-home access" choice for a picked backup, or
 * null when there is nothing to bring. It is off by default: the new server
 * may be on another network, behind another router. An uploaded file can't
 * be looked into first, so the choice says "if it has one".
 */
export function remoteAccessChoice(
  picked:
    | { kind: "folder"; backup: Pick<FolderBackup, "remote_access"> }
    | { kind: "file" }
    | null,
): { label: string; detail: string } | null {
  if (!picked) return null;
  const caveat =
    "Leave this off if the new server is on another network or behind another router; you can set it up again in Settings → Connections.";
  if (picked.kind === "file") {
    return {
      label: "Also restore away-from-home access, if the backup has it",
      detail: caveat,
    };
  }
  const setting = picked.backup.remote_access;
  if (!setting) return null;
  const how =
    setting.mode === "manual"
      ? `the address ${setting.manual_url ?? "it used"}`
      : `a router port forward${setting.external_port ? ` on port ${setting.external_port}` : ""}`;
  return {
    label: "Also restore away-from-home access",
    detail: `The old server was reached away from home through ${how}. ${caveat}`,
  };
}
