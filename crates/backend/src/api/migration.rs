//! Moving a library between servers, Roon-style but guided: scheduled
//! backups that are also copied to a folder or NAS share, restoring one of
//! those backups on a new server (including during first-run setup, before
//! any account exists), and restarting the server to apply it.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use actix_multipart::Multipart;
use actix_web::{HttpRequest, HttpResponse, get, post, put, web};
use chrono::Utc;
use diesel::{QueryDsl, RunQueryDsl};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::data::{
    BACKUP_FORMAT_VERSION, RestoreOptions, backup_directory, safe_backup_filename,
    stage_uploaded_backup, validate_and_stage_restore,
};
use crate::api::error::{bad_request, forbidden, internal_server_error, service_unavailable};
use crate::library::state::LibraryLifecycle;
use crate::persistence::connection::DbPool;
use crate::settings;

const MAX_DESTINATION_PATH_BYTES: usize = 4096;
const MAX_LISTED_BACKUPS: usize = 200;
const DEFAULT_DESTINATION_KEEP: u32 = 10;
const MAX_DESTINATION_KEEP: u32 = 365;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupFrequency {
    #[default]
    Daily,
    Weekly,
    Off,
}

impl BackupFrequency {
    /// Hours between automatic backups, or `None` when they are off.
    pub fn interval_hours(self) -> Option<i64> {
        match self {
            Self::Daily => Some(24),
            Self::Weekly => Some(24 * 7),
            Self::Off => None,
        }
    }
}

/// When automatic backups run and where an extra copy of each one goes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackupSchedule {
    pub frequency: BackupFrequency,
    /// A folder (often a mounted NAS share) that receives a copy of every
    /// backup, so the library survives the loss of this machine.
    pub destination: Option<String>,
    /// How many backups to keep in the destination folder.
    pub destination_keep: u32,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self {
            frequency: BackupFrequency::Daily,
            destination: None,
            destination_keep: DEFAULT_DESTINATION_KEEP,
        }
    }
}

/// The outcome of the last copy to the destination folder.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct DestinationReport {
    pub last_copied_at: Option<String>,
    pub last_copied_file: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScheduleStatus {
    #[serde(flatten)]
    pub schedule: BackupSchedule,
    pub destination_report: DestinationReport,
    /// Where this server keeps its own backups.
    pub local_directory: String,
}

/// A backup found in a folder, read from the manifest inside the archive.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FolderBackup {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub created_at: Option<String>,
    pub library_name: Option<String>,
    pub instance_id: Option<String>,
    pub product_version: Option<String>,
    pub library_roots: Vec<String>,
    /// The old server's away-from-home setting, when it had one. A restore
    /// offers to bring it along (off by default).
    pub remote_access: Option<serde_json::Value>,
}

fn schedule_path(directory: &Path) -> PathBuf {
    directory.join("schedule.json")
}

fn report_path(directory: &Path) -> PathBuf {
    directory.join("destination-report.json")
}

fn load_schedule_from(directory: &Path) -> BackupSchedule {
    fs::read(schedule_path(directory))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BackupSchedule>(&bytes).ok())
        .map(|mut schedule| {
            schedule.destination_keep = schedule.destination_keep.clamp(1, MAX_DESTINATION_KEEP);
            schedule
        })
        .unwrap_or_default()
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes)
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|error| {
            let _ = fs::remove_file(&temporary);
            error.to_string()
        })
}

/// The schedule lives next to the backups (not in `Config`), so restoring a
/// backup from another machine never brings that machine's folder paths.
pub fn load_schedule() -> BackupSchedule {
    load_schedule_from(&backup_directory())
}

fn load_report() -> DestinationReport {
    fs::read(report_path(&backup_directory()))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn schedule_status() -> ScheduleStatus {
    ScheduleStatus {
        schedule: load_schedule(),
        destination_report: load_report(),
        local_directory: backup_directory().to_string_lossy().into_owned(),
    }
}

pub fn record_mirror_result(filename: &str, error: Option<&str>) {
    let mut report = load_report();
    let now = Utc::now().to_rfc3339();
    match error {
        None => {
            report.last_copied_at = Some(now);
            report.last_copied_file = Some(filename.to_string());
            report.last_error = None;
            report.last_error_at = None;
        }
        Some(error) => {
            report.last_error = Some(error.to_string());
            report.last_error_at = Some(now);
        }
    }
    if let Err(error) = write_json_atomically(&report_path(&backup_directory()), &report) {
        tracing::warn!(%error, "could not record the backup copy result");
    }
}

/// Music folders this server indexes (from `Config/libraries.json`).
pub fn configured_library_roots() -> Vec<String> {
    #[derive(Deserialize)]
    struct Libraries {
        paths: Vec<String>,
    }
    fs::read(crate::library::storage::get_libraries_config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Libraries>(&bytes).ok())
        .map(|libraries| libraries.paths)
        .unwrap_or_default()
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Folders that hold the operating system itself. No backup belongs there,
/// and the server never lists or writes them on a request's say-so.
fn is_system_folder(path: &Path) -> bool {
    #[cfg(unix)]
    {
        const SYSTEM: &[&str] = &[
            "/proc",
            "/sys",
            "/dev",
            "/etc",
            "/boot",
            "/run",
            "/bin",
            "/sbin",
            "/lib",
            "/lib32",
            "/lib64",
            "/usr",
            "/System",
            "/private/etc",
        ];
        path == Path::new("/") || SYSTEM.iter().any(|system| path.starts_with(system))
    }
    #[cfg(windows)]
    {
        std::env::var_os("SystemRoot").is_some_and(|root| {
            Path::new(&root)
                .canonicalize()
                .is_ok_and(|root| path.starts_with(root))
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        false
    }
}

/// The trust boundary for a folder an administrator names (a backup
/// destination, a folder to look for backups in, the folder of a backup to
/// restore). The path is resolved once (`..`, symlinks) and only the
/// resolved folder is used afterwards. It must be an existing folder
/// outside the operating system and outside this server's own data folder,
/// except for the server's own backup folder (`backups`).
fn resolve_chosen_folder(value: &str, data_dir: &Path, backups: &Path) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_DESTINATION_PATH_BYTES {
        return Err("Enter a folder.".to_string());
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err(
            "Use a full folder path, for example /mnt/nas/parson-backups or \\\\nas\\backups."
                .to_string(),
        );
    }
    let folder = path
        .canonicalize()
        .ok()
        .filter(|folder| folder.is_dir())
        .ok_or_else(|| {
            format!(
                "{trimmed} isn’t a folder this server can see. If it is a NAS share, check that it is mounted on the server."
            )
        })?;
    let resolved = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let in_backups = folder.starts_with(resolved(backups));
    if folder.starts_with(resolved(data_dir)) && !in_backups {
        return Err(
            "That folder belongs to Parson’s own data. Choose another drive or a NAS share."
                .to_string(),
        );
    }
    if is_system_folder(&folder) {
        return Err("That is a system folder. Choose another drive or a NAS share.".to_string());
    }
    Ok(folder)
}

/// [`resolve_chosen_folder`] against this server's data folder.
fn chosen_folder(value: &str) -> Result<PathBuf, String> {
    resolve_chosen_folder(value, &settings::data_path(&[]), &backup_directory())
}

/// Checks that `value` names an existing folder this server can write to.
fn validate_destination(value: &str, local_backups: &Path) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        return Err("Enter the folder to copy backups to.".to_string());
    }
    let path = resolve_chosen_folder(value, &settings::data_path(&[]), local_backups)?;
    let trimmed = value.trim();
    if same_directory(&path, local_backups) {
        return Err(
            "That is the folder Parson already keeps its own backups in. Choose another drive or a NAS share."
                .to_string(),
        );
    }
    let probe = path.join(format!(".parson-write-check-{}", Uuid::new_v4()));
    fs::write(&probe, b"parson")
        .and_then(|()| fs::remove_file(&probe))
        .map_err(|error| format!("Parson can’t write to {trimmed}: {error}"))?;
    Ok(path)
}

/// Copies one backup (and its metadata sidecar) into `destination`, then
/// keeps only the newest `keep` backups there. Copies land under a temporary
/// name first, so a NAS that drops mid-copy never shows a half backup.
pub fn mirror_backup(
    source_directory: &Path,
    filename: &str,
    destination: &Path,
    keep: u32,
) -> Result<(), String> {
    let filename = safe_backup_filename(filename).ok_or("invalid backup filename")?;
    if !destination.is_dir() {
        return Err(format!(
            "{} is not available. If it is a NAS share, check that it is mounted.",
            destination.display()
        ));
    }
    for name in [filename.to_string(), format!("{filename}.meta.json")] {
        let source = source_directory.join(&name);
        if !source.is_file() {
            continue;
        }
        let temporary = destination.join(format!(".{name}.{}.partial", Uuid::new_v4()));
        let target = destination.join(&name);
        if let Err(error) = fs::copy(&source, &temporary).and_then(|_| {
            File::open(&temporary)?.sync_all()?;
            fs::rename(&temporary, &target)
        }) {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "Could not copy the backup to {}: {error}",
                destination.display()
            ));
        }
    }
    prune_destination(destination, keep.clamp(1, MAX_DESTINATION_KEEP) as usize);
    Ok(())
}

fn backup_files_in(directory: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            safe_backup_filename(&name)?;
            entry
                .metadata()
                .ok()
                .filter(|metadata| metadata.is_file())?;
            Some((name, entry.path()))
        })
        .collect::<Vec<_>>();
    // Names carry a sortable UTC timestamp: newest first.
    files.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    files
}

fn prune_destination(destination: &Path, keep: usize) {
    for (name, path) in backup_files_in(destination).into_iter().skip(keep) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(destination.join(format!("{name}.meta.json")));
    }
}

/// Reads `manifest.json`, the first entry of every Parson backup.
fn read_backup_manifest(path: &Path) -> Result<serde_json::Value, String> {
    let input = File::open(path).map_err(|error| error.to_string())?;
    let decoder = zstd::Decoder::new(input).map_err(|error| error.to_string())?;
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive.entries().map_err(|error| error.to_string())?;
    let mut entry = entries
        .next()
        .ok_or("the backup is empty")?
        .map_err(|error| error.to_string())?;
    if entry.path().map_err(|error| error.to_string())?.as_ref() != Path::new("manifest.json") {
        return Err("the backup has no manifest".to_string());
    }
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(MAX_MANIFEST_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if manifest["format"] != "parson-private-backup" || manifest["version"] != BACKUP_FORMAT_VERSION
    {
        return Err("this is not a supported Parson private backup".to_string());
    }
    Ok(manifest)
}

fn text_field(manifest: &serde_json::Value, key: &str) -> Option<String> {
    manifest[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_string)
}

/// Parson backups in `directory`, newest first, described from their manifests.
/// Files that are not readable Parson backups are skipped.
pub fn list_folder_backups(directory: &Path) -> Vec<FolderBackup> {
    backup_files_in(directory)
        .into_iter()
        .take(MAX_LISTED_BACKUPS)
        .filter_map(|(filename, path)| {
            let manifest = read_backup_manifest(&path).ok()?;
            Some(FolderBackup {
                size_bytes: path.metadata().map(|metadata| metadata.len()).unwrap_or(0),
                created_at: text_field(&manifest, "created_at"),
                library_name: text_field(&manifest, "library_name"),
                instance_id: text_field(&manifest, "instance_id")
                    .filter(|value| Uuid::parse_str(value).is_ok()),
                product_version: text_field(&manifest, "product_version"),
                remote_access: crate::api::data::remote_access_setting(
                    manifest["remote_access"].to_string().as_bytes(),
                ),
                library_roots: manifest["library_roots"]
                    .as_array()
                    .map(|roots| {
                        roots
                            .iter()
                            .filter_map(|root| root.as_str().map(str::to_string))
                            .take(64)
                            .collect()
                    })
                    .unwrap_or_default(),
                path: path.to_string_lossy().into_owned(),
                filename,
            })
        })
        .collect()
}

/// Folders worth looking in for a backup on a fresh server: its own backup
/// folder (a reinstall), the scheduled destination, and the conventional
/// `/backups` mount of the Docker image.
fn suggested_backup_folders() -> Vec<String> {
    let mut folders = vec![backup_directory()];
    if let Some(destination) = load_schedule().destination {
        folders.push(PathBuf::from(destination));
    }
    if settings::is_container() {
        folders.push(PathBuf::from("/backups"));
    }
    let mut seen = std::collections::HashSet::new();
    folders
        .into_iter()
        .filter(|folder| folder.is_dir())
        .map(|folder| folder.to_string_lossy().into_owned())
        .filter(|folder| seen.insert(folder.clone()))
        .collect()
}

/// Resolves a backup the client picked from a folder listing: a Parson
/// backup file, in a folder [`chosen_folder`] accepts.
fn backup_file_from_request(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value.trim());
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Choose a Parson backup file.")?;
    if !path.is_absolute() || safe_backup_filename(name).is_none() {
        return Err("Choose a Parson backup file (parson-backup-….tar.zst).".to_string());
    }
    let folder = path
        .parent()
        .and_then(Path::to_str)
        .ok_or("Choose a Parson backup file.")?;
    let file = chosen_folder(folder)?.join(name);
    if !file.is_file() {
        return Err("That backup is no longer there.".to_string());
    }
    Ok(file)
}

#[derive(Deserialize)]
struct FolderQuery {
    #[serde(default)]
    path: String,
}

#[derive(Serialize)]
struct FolderListing {
    path: Option<String>,
    backups: Vec<FolderBackup>,
    suggested_folders: Vec<String>,
}

/// Lists backups. `exclude_instance` hides this server's own backups during
/// first-run setup, where restoring a fresh, empty server onto itself is
/// never what anyone wants.
async fn folder_listing(path: String, exclude_instance: Option<String>) -> HttpResponse {
    let result = web::block(move || {
        let trimmed = path.trim().to_string();
        let folders = suggested_backup_folders();
        let mut backups = Vec::new();
        if trimmed.is_empty() {
            for folder in &folders {
                backups.extend(list_folder_backups(Path::new(folder)));
            }
            backups.sort_by(|left, right| right.filename.cmp(&left.filename));
            backups.dedup_by(|left, right| left.filename == right.filename);
        } else if let Ok(folder) = chosen_folder(&trimmed) {
            backups = list_folder_backups(&folder);
        }
        if let Some(own) = exclude_instance.as_deref() {
            backups.retain(|backup| backup.instance_id.as_deref() != Some(own));
        }
        FolderListing {
            path: (!trimmed.is_empty()).then_some(trimmed),
            backups,
            suggested_folders: folders,
        }
    })
    .await;
    match result {
        Ok(listing) => HttpResponse::Ok().json(listing),
        Err(error) => {
            tracing::error!(%error, "backup folder listing worker failed");
            internal_server_error("Could not look for backups.", "backup_listing_failed")
        }
    }
}

#[derive(Deserialize)]
struct RestoreFromFolderRequest {
    path: String,
    #[serde(default)]
    adopt_identity: bool,
    #[serde(default)]
    restore_remote_access: bool,
}

async fn stage_backup_from_folder(request: RestoreFromFolderRequest) -> Result<(), HttpResponse> {
    let path = backup_file_from_request(&request.path)
        .map_err(|message| bad_request(message, "invalid_backup"))?;
    let options = RestoreOptions {
        adopt_identity: request.adopt_identity,
        restore_remote_access: request.restore_remote_access,
    };
    match web::block(move || validate_and_stage_restore(&path, options)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(bad_request(error, "invalid_backup")),
        Err(error) => {
            tracing::error!(%error, "restore validation worker failed");
            Err(internal_server_error(
                "Could not validate the backup.",
                "restore_failed",
            ))
        }
    }
}

fn restarting_response() -> HttpResponse {
    crate::server::request_restart();
    HttpResponse::Accepted().json(serde_json::json!({
        "restart_required": false,
        "restarting": true,
        "message": "Backup validated. Parson is restarting to restore it."
    }))
}

#[get("/backups/schedule")]
async fn get_schedule() -> HttpResponse {
    match web::block(schedule_status).await {
        Ok(status) => HttpResponse::Ok().json(status),
        Err(error) => {
            tracing::error!(%error, "backup schedule worker failed");
            internal_server_error(
                "Could not read the backup schedule.",
                "backup_schedule_failed",
            )
        }
    }
}

#[derive(Deserialize)]
struct ScheduleUpdate {
    frequency: BackupFrequency,
    #[serde(default)]
    destination: Option<String>,
    #[serde(default)]
    destination_keep: Option<u32>,
}

#[put("/backups/schedule")]
async fn put_schedule(update: web::Json<ScheduleUpdate>) -> HttpResponse {
    let update = update.into_inner();
    let result = web::block(move || -> Result<ScheduleStatus, String> {
        let local = backup_directory();
        fs::create_dir_all(&local).map_err(|error| error.to_string())?;
        let destination = match update.destination.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(value) => Some(
                validate_destination(value, &local)?
                    .to_string_lossy()
                    .into_owned(),
            ),
        };
        let schedule = BackupSchedule {
            frequency: update.frequency,
            destination,
            destination_keep: update
                .destination_keep
                .unwrap_or(DEFAULT_DESTINATION_KEEP)
                .clamp(1, MAX_DESTINATION_KEEP),
        };
        write_json_atomically(&schedule_path(&local), &schedule)?;
        Ok(schedule_status())
    })
    .await;
    match result {
        Ok(Ok(status)) => HttpResponse::Ok().json(status),
        Ok(Err(message)) => bad_request(message, "invalid_backup_destination"),
        Err(error) => {
            tracing::error!(%error, "backup schedule worker failed");
            internal_server_error(
                "Could not save the backup schedule.",
                "backup_schedule_failed",
            )
        }
    }
}

#[get("/backups/folder")]
async fn admin_folder_listing(query: web::Query<FolderQuery>) -> HttpResponse {
    folder_listing(query.into_inner().path, None).await
}

#[post("/restore/from-folder")]
async fn admin_restore_from_folder(request: web::Json<RestoreFromFolderRequest>) -> HttpResponse {
    match stage_backup_from_folder(request.into_inner()).await {
        Ok(()) => HttpResponse::Accepted().json(serde_json::json!({
            "restart_required": true,
            "message": "Backup validated. Restart Parson to apply it."
        })),
        Err(response) => response,
    }
}

#[post("/restart")]
async fn admin_restart() -> HttpResponse {
    crate::server::request_restart();
    HttpResponse::Accepted().json(serde_json::json!({
        "restarting": true,
        "message": "Parson is restarting."
    }))
}

pub fn configure_admin(cfg: &mut web::ServiceConfig) {
    cfg.service(get_schedule)
        .service(put_schedule)
        .service(admin_folder_listing)
        .service(admin_restore_from_folder)
        .service(admin_restart);
}

/// Who may restore during setup: anyone while the server has no account at
/// all (the same trust as creating the first administrator), otherwise only
/// an administrator while the library still needs setting up.
async fn setup_restore_allowed(
    request: &HttpRequest,
    lifecycle: &LibraryLifecycle,
    pool: DbPool,
) -> Result<bool, HttpResponse> {
    let lookup_pool = pool.clone();
    let user_count = web::block(move || -> Result<i64, String> {
        use crate::persistence::schema::user::dsl::user;
        let mut connection = lookup_pool.get().map_err(|error| error.to_string())?;
        user.count()
            .get_result(&mut connection)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result)
    .map_err(|error| {
        tracing::error!(%error, "setup restore lookup failed");
        service_unavailable(
            "Setup is temporarily unavailable.",
            "setup_status_unavailable",
        )
    })?;
    if user_count == 0 {
        return Ok(true);
    }
    let state = lifecycle.readiness().await.state;
    if !matches!(
        state,
        crate::library::state::LibraryReadinessState::NoLibraryIndexed
            | crate::library::state::LibraryReadinessState::Failed
    ) {
        return Ok(false);
    }
    crate::api::auth::request_has_current_admin(request, pool)
        .await
        .map_err(|error| {
            tracing::error!(%error, "setup restore authorization failed");
            service_unavailable(
                "Setup authorization is temporarily unavailable.",
                "setup_authorization_unavailable",
            )
        })
}

fn setup_restore_forbidden() -> HttpResponse {
    forbidden(
        "This server is already set up. Restore backups from Settings → Data instead.",
        "setup_unavailable",
    )
}

async fn setup_backups(
    request: HttpRequest,
    query: web::Query<FolderQuery>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    match setup_restore_allowed(&request, lifecycle.get_ref(), pool.get_ref().clone()).await {
        Ok(true) => folder_listing(query.into_inner().path, settings::instance_id().ok()).await,
        Ok(false) => setup_restore_forbidden(),
        Err(response) => response,
    }
}

async fn setup_restore_upload(
    request: HttpRequest,
    multipart: Multipart,
    options: web::Query<RestoreOptions>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    match setup_restore_allowed(&request, lifecycle.get_ref(), pool.get_ref().clone()).await {
        Ok(true) => {}
        Ok(false) => return setup_restore_forbidden(),
        Err(response) => return response,
    }
    match stage_uploaded_backup(multipart, options.into_inner()).await {
        Ok(()) => restarting_response(),
        Err(response) => response,
    }
}

async fn setup_restore_from_folder(
    request: HttpRequest,
    body: web::Json<RestoreFromFolderRequest>,
    lifecycle: web::Data<LibraryLifecycle>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    match setup_restore_allowed(&request, lifecycle.get_ref(), pool.get_ref().clone()).await {
        Ok(true) => {}
        Ok(false) => return setup_restore_forbidden(),
        Err(response) => return response,
    }
    match stage_backup_from_folder(body.into_inner()).await {
        Ok(()) => restarting_response(),
        Err(response) => response,
    }
}

pub fn configure_setup(cfg: &mut web::ServiceConfig) {
    cfg.route("/backups", web::get().to(setup_backups));
    cfg.route("/restore", web::post().to(setup_restore_upload));
    cfg.route(
        "/restore/from-folder",
        web::post().to(setup_restore_from_folder),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        BackupFrequency, BackupSchedule, backup_file_from_request, list_folder_backups,
        load_schedule_from, mirror_backup, prune_destination, resolve_chosen_folder, schedule_path,
        validate_destination,
    };
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("parson-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_backup(directory: &Path, filename: &str, manifest: serde_json::Value) {
        let output = fs::File::create(directory.join(filename)).unwrap();
        let encoder = zstd::Encoder::new(output, 1).unwrap();
        let mut archive = tar::Builder::new(encoder);
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        archive
            .append_data(&mut header, "manifest.json", Cursor::new(bytes))
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
    }

    fn manifest(name: &str) -> serde_json::Value {
        serde_json::json!({
            "format": "parson-private-backup",
            "version": 1,
            "created_at": "2026-10-20T08:00:00+00:00",
            "product_version": "1.0.1",
            "library_name": name,
            "instance_id": "33333333-3333-4333-8333-333333333333",
            "library_roots": ["/home/ana/Music"],
        })
    }

    #[actix_web::test]
    async fn schedule_and_folder_routes_are_not_taken_for_backup_downloads() {
        use actix_web::{App, test as actix_test, web};
        let app = actix_test::init_service(
            App::new()
                .service(web::scope("/data/admin").configure(crate::api::data::configure_admin)),
        )
        .await;
        let schedule: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/data/admin/backups/schedule")
                .to_request(),
        )
        .await;
        assert!(schedule.get("frequency").is_some(), "{schedule}");
        let listing: serde_json::Value = actix_test::call_and_read_body_json(
            &app,
            actix_test::TestRequest::get()
                .uri("/data/admin/backups/folder")
                .to_request(),
        )
        .await;
        assert!(listing.get("backups").is_some(), "{listing}");
    }

    #[test]
    fn the_schedule_defaults_to_daily_and_survives_bad_files() {
        let directory = scratch("schedule");
        assert_eq!(load_schedule_from(&directory), BackupSchedule::default());
        fs::write(schedule_path(&directory), b"{not json").unwrap();
        assert_eq!(load_schedule_from(&directory), BackupSchedule::default());
        fs::write(
            schedule_path(&directory),
            br#"{"frequency":"weekly","destination":"/mnt/nas","destination_keep":0}"#,
        )
        .unwrap();
        let schedule = load_schedule_from(&directory);
        assert_eq!(schedule.frequency, BackupFrequency::Weekly);
        assert_eq!(schedule.destination.as_deref(), Some("/mnt/nas"));
        assert_eq!(schedule.destination_keep, 1);
        assert_eq!(BackupFrequency::Weekly.interval_hours(), Some(168));
        assert_eq!(BackupFrequency::Off.interval_hours(), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn destinations_must_be_existing_writable_folders_elsewhere() {
        let local = scratch("local-backups");
        let nas = scratch("nas");
        assert!(validate_destination("", &local).is_err());
        assert!(
            validate_destination("relative/folder", &local)
                .unwrap_err()
                .contains("full folder path")
        );
        assert!(
            validate_destination(&nas.join("missing").to_string_lossy(), &local)
                .unwrap_err()
                .contains("mounted")
        );
        assert!(
            validate_destination(&local.to_string_lossy(), &local)
                .unwrap_err()
                .contains("already keeps")
        );
        assert_eq!(
            validate_destination(&nas.to_string_lossy(), &local).unwrap(),
            nas
        );
        // The write check leaves nothing behind.
        assert_eq!(fs::read_dir(&nas).unwrap().count(), 0);
        fs::remove_dir_all(local).unwrap();
        fs::remove_dir_all(nas).unwrap();
    }

    #[test]
    fn mirrored_backups_are_complete_and_pruned_to_the_newest() {
        let local = scratch("mirror-local");
        let nas = scratch("mirror-nas");
        let names = [
            "parson-backup-20261001T000000000Z.tar.zst",
            "parson-backup-20261002T000000000Z.tar.zst",
            "parson-backup-20261003T000000000Z.tar.zst",
        ];
        for name in names {
            write_backup(&local, name, manifest("Home"));
            fs::write(local.join(format!("{name}.meta.json")), b"{}").unwrap();
            mirror_backup(&local, name, &nas, 2).unwrap();
        }
        let mut remaining = fs::read_dir(&nas)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        remaining.sort();
        assert_eq!(
            remaining,
            vec![
                names[1].to_string(),
                format!("{}.meta.json", names[1]),
                names[2].to_string(),
                format!("{}.meta.json", names[2]),
            ]
        );
        assert!(mirror_backup(&local, "../escape.tar.zst", &nas, 2).is_err());
        assert!(
            mirror_backup(&local, names[0], &nas.join("unmounted"), 2)
                .unwrap_err()
                .contains("mounted")
        );
        prune_destination(&nas, 1);
        assert_eq!(list_folder_backups(&nas).len(), 1);
        fs::remove_dir_all(local).unwrap();
        fs::remove_dir_all(nas).unwrap();
    }

    #[test]
    fn folder_listings_show_the_away_from_home_setting_a_restore_can_bring() {
        let folder = scratch("listing-remote");
        let mut with_remote = manifest("Basement NAS");
        with_remote["remote_access"] = serde_json::json!({
            "mode": "router", "manual_url": null, "external_port": 41993
        });
        write_backup(
            &folder,
            "parson-backup-20261020T080000000Z.tar.zst",
            with_remote,
        );
        let backups = list_folder_backups(&folder);
        let remote = backups[0].remote_access.as_ref().expect("remote access");
        assert_eq!(remote["mode"], "router");
        assert_eq!(remote["external_port"], 41993);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn folder_listings_describe_backups_from_their_manifests() {
        let folder = scratch("listing");
        write_backup(
            &folder,
            "parson-backup-20261020T080000000Z.tar.zst",
            manifest("Studio"),
        );
        write_backup(
            &folder,
            "parson-backup-20261021T080000000Z.tar.zst",
            serde_json::json!({"format": "something-else", "version": 1}),
        );
        fs::write(folder.join("notes.txt"), b"not a backup").unwrap();
        let backups = list_folder_backups(&folder);
        assert_eq!(backups.len(), 1);
        let backup = &backups[0];
        assert_eq!(backup.library_name.as_deref(), Some("Studio"));
        assert_eq!(
            backup.instance_id.as_deref(),
            Some("33333333-3333-4333-8333-333333333333")
        );
        assert_eq!(backup.library_roots, vec!["/home/ana/Music".to_string()]);
        assert!(backup.size_bytes > 0);
        assert_eq!(backup.remote_access, None);

        let picked = backup_file_from_request(&backup.path).unwrap();
        assert_eq!(picked, PathBuf::from(&backup.path).canonicalize().unwrap());
        // The same file named through `..` resolves to the same place.
        let detour = folder
            .join("..")
            .join(folder.file_name().unwrap())
            .join(&backup.filename);
        assert_eq!(
            backup_file_from_request(&detour.to_string_lossy()).unwrap(),
            picked
        );
        assert!(backup_file_from_request(&folder.join("notes.txt").to_string_lossy()).is_err());
        assert!(backup_file_from_request("parson-backup-20261020T080000000Z.tar.zst").is_err());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn chosen_folders_resolve_once_and_stay_out_of_parson_data_and_the_system() {
        let data = scratch("chosen-data");
        let backups = data.join("Backups");
        fs::create_dir_all(&backups).unwrap();
        fs::create_dir_all(data.join("Config")).unwrap();
        let nas = scratch("chosen-nas");
        let resolve = |value: &str| resolve_chosen_folder(value, &data, &backups);
        let canonical = |path: &Path| path.canonicalize().unwrap();

        // An ordinary folder elsewhere resolves to its canonical form.
        assert_eq!(resolve(&nas.to_string_lossy()).unwrap(), canonical(&nas));
        let detour = nas.join("..").join(nas.file_name().unwrap()).join(".");
        assert_eq!(resolve(&detour.to_string_lossy()).unwrap(), canonical(&nas));

        // `..` cannot walk into the server's own data folder.
        let into_data = nas
            .join("..")
            .join(data.file_name().unwrap())
            .join("Config");
        assert!(
            resolve(&into_data.to_string_lossy())
                .unwrap_err()
                .contains("own data")
        );
        assert!(resolve(&data.to_string_lossy()).is_err());

        // Neither can a symlink planted elsewhere.
        #[cfg(unix)]
        {
            let link = nas.join("innocent-looking");
            std::os::unix::fs::symlink(data.join("Config"), &link).unwrap();
            assert!(
                resolve(&link.to_string_lossy())
                    .unwrap_err()
                    .contains("own data")
            );
            // A link to the server's own backups is fine and resolves there.
            let to_backups = nas.join("backups-link");
            std::os::unix::fs::symlink(&backups, &to_backups).unwrap();
            assert_eq!(
                resolve(&to_backups.to_string_lossy()).unwrap(),
                canonical(&backups)
            );
            // System folders, however they are spelled.
            for system in ["/", "/etc", "/usr/../etc", "/proc/self/..", "/dev/."] {
                assert!(
                    resolve(system).unwrap_err().contains("system folder"),
                    "{system}"
                );
            }
        }

        // The server's own backup folder is where a reinstall looks.
        assert_eq!(
            resolve(&backups.to_string_lossy()).unwrap(),
            canonical(&backups)
        );
        // Only existing, absolute folders.
        assert!(resolve("").is_err());
        assert!(resolve("relative/folder").is_err());
        assert!(resolve(&nas.join("missing").to_string_lossy()).is_err());
        fs::write(nas.join("file"), b"x").unwrap();
        assert!(resolve(&nas.join("file").to_string_lossy()).is_err());
        assert!(resolve(&format!("/{}", "a".repeat(5000))).is_err());
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(nas).unwrap();
    }

    #[actix_web::test]
    async fn only_administrators_reach_the_folder_routes() {
        use actix_web::{App, http::StatusCode, test as actix_test, web};
        use actix_web_httpauth::middleware::HttpAuthentication;
        let app = actix_test::init_service(
            App::new().service(
                web::scope("/data/admin")
                    .wrap(HttpAuthentication::with_fn(crate::api::auth::admin_guard))
                    .configure(crate::api::data::configure_admin),
            ),
        )
        .await;
        let listener =
            crate::api::auth::generate_access_token(7, "listener", 0, "user", 0).unwrap();
        for token in [None, Some(listener.as_str())] {
            for request in [
                actix_test::TestRequest::get().uri("/data/admin/backups/folder?path=/etc"),
                actix_test::TestRequest::post()
                    .uri("/data/admin/restore/from-folder")
                    .set_json(serde_json::json!({"path": "/etc/parson-backup-x.tar.zst"})),
                actix_test::TestRequest::put()
                    .uri("/data/admin/backups/schedule")
                    .set_json(serde_json::json!({"frequency": "daily", "destination": "/etc"})),
            ] {
                let request = match token {
                    Some(token) => {
                        request.insert_header(("Authorization", format!("Bearer {token}")))
                    }
                    None => request,
                };
                let response = actix_test::call_service(&app, request.to_request()).await;
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
    }
}
