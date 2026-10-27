import api from "../core/http";

export interface BackupSummary {
  filename: string;
  created_at: string;
  size_bytes: number;
  tier?: "daily" | "weekly" | "monthly" | null;
  /** The folder or NAS share the scheduled copy went to. */
  copied_to?: string;
  /** Why the copy to the folder or NAS share failed. */
  copy_error?: string;
}

export type BackupFrequency = "daily" | "weekly" | "off";

export interface BackupSchedule {
  frequency: BackupFrequency;
  /** A folder (often a mounted NAS share) that receives a copy of each backup. */
  destination: string | null;
  destination_keep: number;
}

export interface BackupScheduleStatus extends BackupSchedule {
  destination_report: {
    last_copied_at: string | null;
    last_copied_file: string | null;
    last_error: string | null;
    last_error_at: string | null;
  };
  /** Where this server keeps its own backups. */
  local_directory: string;
}

/** A backup found in a folder, described by the manifest inside it. */
export interface FolderBackup {
  filename: string;
  path: string;
  size_bytes: number;
  created_at: string | null;
  library_name: string | null;
  instance_id: string | null;
  product_version: string | null;
  /** Music folders on the server the backup came from. */
  library_roots: string[];
  /**
   * The old server's away-from-home setting, when it had one; a restore can
   * bring it along. Older servers and backups omit it.
   */
  remote_access?: BackupRemoteAccess | null;
}

export interface BackupRemoteAccess {
  mode: "router" | "manual";
  manual_url: string | null;
  external_port: number | null;
}

export interface FolderBackupListing {
  path: string | null;
  backups: FolderBackup[];
  /** Folders the server looked in when no path was given. */
  suggested_folders: string[];
}

export interface DataStatus {
  database_bytes: number;
  user_file_bytes: number;
  cache_bytes: number;
  listen_history_items: number;
  favorite_items: number;
  playlist_items: number;
  backups: BackupSummary[];
  /** Null when automatic backups are off. */
  automatic_backup_interval_hours: number | null;
  schedule: BackupScheduleStatus;
  daily_backup_count: number;
  weekly_backup_count: number;
  monthly_backup_count: number;
  restore_pending: boolean;
  reset_pending: boolean;
}

export interface RestoreResult {
  restart_required: boolean;
  /** True when the server restarts on its own to apply the backup. */
  restarting?: boolean;
  message: string;
}

export interface RestoreOptions {
  /**
   * Take over the backed-up library's identity, so phones and computers that
   * knew the old server treat this one as the same library.
   */
  adoptIdentity?: boolean;
  /**
   * Also bring the old server's away-from-home setting. Off unless asked:
   * the new server may be on another network, behind another router.
   */
  restoreRemoteAccess?: boolean;
}

export async function getDataStatus(): Promise<DataStatus> {
  return (await api.get<DataStatus>("/data/admin/status")).data;
}

export async function createPrivateBackup(): Promise<BackupSummary> {
  return (
    await api.post<BackupSummary>("/data/admin/backups", undefined, {
      timeout: 30 * 60 * 1000,
    })
  ).data;
}

export async function downloadPrivateBackup(filename: string): Promise<Blob> {
  return (
    await api.get<Blob>(`/data/admin/backups/${encodeURIComponent(filename)}`, {
      responseType: "blob",
      timeout: 30 * 60 * 1000,
    })
  ).data;
}

export async function deletePrivateBackup(filename: string): Promise<void> {
  await api.delete(`/data/admin/backups/${encodeURIComponent(filename)}`);
}

export function restoreQuery(options: RestoreOptions = {}) {
  const query = new URLSearchParams();
  if (options.adoptIdentity) query.set("adopt_identity", "true");
  if (options.restoreRemoteAccess) query.set("restore_remote_access", "true");
  const text = query.toString();
  return text ? `?${text}` : "";
}

function restoreBody(path: string, options: RestoreOptions) {
  return {
    path,
    adopt_identity: Boolean(options.adoptIdentity),
    restore_remote_access: Boolean(options.restoreRemoteAccess),
  };
}

export async function restorePrivateBackup(
  file: File,
  options: RestoreOptions = {},
): Promise<RestoreResult> {
  const data = new FormData();
  data.append("backup", file, file.name);
  return (
    await api.post<RestoreResult>(
      `/data/admin/restore${restoreQuery(options)}`,
      data,
      { timeout: 30 * 60 * 1000 },
    )
  ).data;
}

export async function getBackupSchedule(): Promise<BackupScheduleStatus> {
  return (await api.get<BackupScheduleStatus>("/data/admin/backups/schedule"))
    .data;
}

export async function updateBackupSchedule(
  schedule: Pick<BackupSchedule, "frequency"> &
    Partial<Pick<BackupSchedule, "destination" | "destination_keep">>,
): Promise<BackupScheduleStatus> {
  return (
    await api.put<BackupScheduleStatus>(
      "/data/admin/backups/schedule",
      schedule,
    )
  ).data;
}

/** Backups in `path`, or in the folders the server knows about. */
export async function listFolderBackups(
  path = "",
): Promise<FolderBackupListing> {
  return (
    await api.get<FolderBackupListing>("/data/admin/backups/folder", {
      params: { path },
      timeout: 60_000,
    })
  ).data;
}

export async function restoreBackupFromFolder(
  path: string,
  options: RestoreOptions = {},
): Promise<RestoreResult> {
  return (
    await api.post<RestoreResult>(
      "/data/admin/restore/from-folder",
      restoreBody(path, options),
      { timeout: 30 * 60 * 1000 },
    )
  ).data;
}

/** Restarts the server (for example to apply a staged restore). */
export async function restartServer(): Promise<{ restarting: boolean }> {
  return (await api.post<{ restarting: boolean }>("/data/admin/restart")).data;
}

/**
 * First-run restore: on a server with no accounts yet, a backup from the old
 * server can be restored before anything else. The server restarts itself.
 */
export async function listSetupBackups(
  path = "",
): Promise<FolderBackupListing> {
  return (
    await api.get<FolderBackupListing>("/setup/backups", {
      params: { path },
      timeout: 60_000,
    })
  ).data;
}

export async function restoreSetupBackup(
  file: File,
  options: RestoreOptions = {},
): Promise<RestoreResult> {
  const data = new FormData();
  data.append("backup", file, file.name);
  return (
    await api.post<RestoreResult>(
      `/setup/restore${restoreQuery(options)}`,
      data,
      { timeout: 30 * 60 * 1000 },
    )
  ).data;
}

export async function restoreSetupBackupFromFolder(
  path: string,
  options: RestoreOptions = {},
): Promise<RestoreResult> {
  return (
    await api.post<RestoreResult>(
      "/setup/restore/from-folder",
      restoreBody(path, options),
      { timeout: 30 * 60 * 1000 },
    )
  ).data;
}

export async function exportPublicLibraryIndex(): Promise<Blob> {
  return (
    await api.get<Blob>("/data/admin/public-index", {
      responseType: "blob",
      timeout: 30 * 60 * 1000,
    })
  ).data;
}

export async function exportPersonalData(): Promise<Blob> {
  return (
    await api.get<Blob>("/data/me/export", {
      responseType: "blob",
      timeout: 30 * 60 * 1000,
    })
  ).data;
}

export async function clearPersonalHistory(): Promise<{ deleted: number }> {
  return (await api.delete<{ deleted: number }>("/data/me/history")).data;
}

export async function clearParsonCache(): Promise<{ cleared_bytes: number }> {
  return (
    await api.delete<{ cleared_bytes: number }>("/data/admin/cache", {
      timeout: 30 * 60 * 1000,
    })
  ).data;
}

export async function resetParson(
  confirmation: string,
): Promise<RestoreResult> {
  return (
    await api.post<RestoreResult>("/data/admin/reset", {
      confirmation,
    })
  ).data;
}
