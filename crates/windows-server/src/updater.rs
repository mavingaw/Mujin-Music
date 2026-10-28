use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The releases page this build updates from: `MUJIN_RELEASES_BASE` at
/// build time, such as `https://git.example.com/owner/repo/releases`.
/// tools/releases-base.cjs derives the desktop app's URLs by the same rules.
const DEFAULT_RELEASES_BASE: &str = "https://github.com/mavingaw/Mujin-Music/releases";
const CONFIGURED_RELEASES_BASE: &str = match option_env!("MUJIN_RELEASES_BASE") {
    Some(base) => base,
    None => DEFAULT_RELEASES_BASE,
};
const MAX_UPDATE_BYTES: u64 = 128 * 1024 * 1024;
const SYNCHRONIZE_PROCESS: u32 = 0x0010_0000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct UpdateManifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    Current,
    Available(UpdateManifest),
}

fn releases_base() -> &'static str {
    let base = CONFIGURED_RELEASES_BASE.trim().trim_end_matches('/');
    if base.is_empty() {
        DEFAULT_RELEASES_BASE
    } else {
        base
    }
}

/// The folder holding the newest stable release's files: GitHub serves it
/// at `<base>/latest/download`, Gitea and Forgejo at `<base>/download/latest`.
fn latest_download_base(base: &str) -> String {
    if base.starts_with("https://github.com/") {
        format!("{base}/latest/download")
    } else {
        format!("{base}/download/latest")
    }
}

/// Every official update is a file of a tagged release under this prefix.
fn release_download_prefix(base: &str) -> String {
    format!("{base}/download/")
}

fn default_manifest_url(base: &str) -> String {
    format!("{}/windows-server-update.json", latest_download_base(base))
}

pub fn manifest_url() -> String {
    std::env::var("PARSON_UPDATE_MANIFEST_URL")
        .unwrap_or_else(|_| default_manifest_url(releases_base()))
}

pub fn check(client: &Client, manifest_url: &str) -> Result<UpdateCheck, String> {
    let response = client
        .get(manifest_url)
        .header("accept", "application/json")
        .header("user-agent", format!("ParsonMusicServer/{CURRENT_VERSION}"))
        .send()
        .map_err(|error| format!("could not check for updates: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateCheck::Current);
    }
    let manifest = response
        .error_for_status()
        .map_err(|error| format!("update service returned an error: {error}"))?
        .json::<UpdateManifest>()
        .map_err(|error| format!("invalid update manifest: {error}"))?;
    validate_manifest(&manifest, manifest_url)?;

    let current = Version::parse(CURRENT_VERSION)
        .map_err(|error| format!("invalid installed version: {error}"))?;
    let available = Version::parse(&manifest.version)
        .map_err(|error| format!("invalid available version: {error}"))?;
    if available <= current {
        Ok(UpdateCheck::Current)
    } else {
        Ok(UpdateCheck::Available(manifest))
    }
}

pub fn download(
    client: &Client,
    manifest: &UpdateManifest,
    updates_dir: &Path,
) -> Result<PathBuf, String> {
    fs::create_dir_all(updates_dir)
        .map_err(|error| format!("could not create update folder: {error}"))?;
    let final_path = updates_dir.join(format!("ParsonMusicServer-{}.exe", manifest.version));
    let partial_path = final_path.with_extension("exe.part");
    let _ = fs::remove_file(&partial_path);

    let result = download_to(client, manifest, &partial_path);
    if let Err(error) = result {
        let _ = fs::remove_file(&partial_path);
        return Err(error);
    }
    if final_path.exists() {
        fs::remove_file(&final_path)
            .map_err(|error| format!("could not replace cached update: {error}"))?;
    }
    fs::rename(&partial_path, &final_path)
        .map_err(|error| format!("could not finalize update download: {error}"))?;
    Ok(final_path)
}

fn download_to(
    client: &Client,
    manifest: &UpdateManifest,
    destination: &Path,
) -> Result<(), String> {
    let mut response = client
        .get(&manifest.url)
        .header("user-agent", format!("ParsonMusicServer/{CURRENT_VERSION}"))
        .send()
        .map_err(|error| format!("could not download update: {error}"))?
        .error_for_status()
        .map_err(|error| format!("update download failed: {error}"))?;
    if let Some(length) = response.content_length()
        && length != manifest.size
    {
        return Err(format!(
            "update size changed (expected {}, received {length})",
            manifest.size
        ));
    }

    let mut file = File::create(destination)
        .map_err(|error| format!("could not create update file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| format!("could not read update download: {error}"))?;
        if count == 0 {
            break;
        }
        received = received.saturating_add(count as u64);
        if received > manifest.size || received > MAX_UPDATE_BYTES {
            return Err("update download exceeded its declared size".to_string());
        }
        hasher.update(&buffer[..count]);
        file.write_all(&buffer[..count])
            .map_err(|error| format!("could not write update file: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("could not flush update file: {error}"))?;
    if received != manifest.size {
        return Err(format!(
            "incomplete update download (expected {}, received {received})",
            manifest.size
        ));
    }
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err("update checksum verification failed".to_string());
    }
    Ok(())
}

pub fn client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10 * 60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| format!("could not initialize updater: {error}"))
}

fn validate_manifest(manifest: &UpdateManifest, manifest_url: &str) -> Result<(), String> {
    let custom_channel = std::env::var_os("PARSON_UPDATE_MANIFEST_URL").is_some();
    validate_manifest_from(manifest, manifest_url, releases_base(), custom_channel)
}

fn validate_manifest_from(
    manifest: &UpdateManifest,
    manifest_url: &str,
    releases_base: &str,
    custom_channel: bool,
) -> Result<(), String> {
    Version::parse(&manifest.version).map_err(|_| "update version is invalid".to_string())?;
    if manifest.size == 0 || manifest.size > MAX_UPDATE_BYTES {
        return Err("update size is outside the allowed range".to_string());
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("update checksum is invalid".to_string());
    }
    if !custom_channel && !manifest_url.starts_with("https://") {
        return Err("the update manifest must use HTTPS".to_string());
    }
    if !custom_channel {
        let prefix = release_download_prefix(releases_base);
        let official = prefix.starts_with("https://")
            && manifest
                .url
                .strip_prefix(&prefix)
                .is_some_and(is_release_file_path);
        if !official {
            return Err("the update download is not from the official release channel".to_string());
        }
    }
    Ok(())
}

/// `<tag>/<file>` below the release download prefix, with nothing that
/// could lead a client to another path once the URL is normalized.
fn is_release_file_path(rest: &str) -> bool {
    let segments: Vec<&str> = rest.split('/').collect();
    segments.len() == 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && *segment != "."
                && *segment != ".."
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
        })
}

pub fn start_apply_helper(update: &Path, target: &Path) -> Result<(), String> {
    Command::new(update)
        .arg("--apply-update")
        .arg(target)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .spawn()
        .map_err(|error| format!("could not start the update installer: {error}"))?;
    Ok(())
}

pub fn apply_update(target: &Path, parent_pid: u32) -> Result<(), String> {
    let source = std::env::current_exe()
        .map_err(|error| format!("could not locate downloaded update: {error}"))?;
    wait_for_process(parent_pid)?;
    match apply_update_after_shutdown(&source, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let relaunch = Command::new(target).arg("--background").spawn();
            match relaunch {
                Ok(_) => Err(error),
                Err(relaunch_error) => Err(format!(
                    "{error}; the previous server also could not be relaunched: {relaunch_error}"
                )),
            }
        }
    }
}

fn apply_update_after_shutdown(source: &Path, target: &Path) -> Result<(), String> {
    let temporary = target.with_extension("exe.update-new");
    let backup = target.with_extension("exe.update-backup");
    let _ = fs::remove_file(&temporary);
    fs::copy(source, &temporary)
        .map_err(|error| format!("could not stage the new executable: {error}"))?;
    let _ = fs::remove_file(&backup);
    fs::rename(target, &backup)
        .map_err(|error| format!("could not preserve the installed executable: {error}"))?;
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::rename(&backup, target);
        return Err(format!("could not install the update: {error}"));
    }

    let handshake = source.with_extension("update-ready");
    let _ = fs::remove_file(&handshake);
    let child = Command::new(target)
        .arg("--background")
        .arg("--update-handshake")
        .arg(&handshake)
        .arg("--cleanup-update")
        .arg(source)
        .arg("--cleanup-backup")
        .arg(&backup)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return rollback(target, &backup, format!("relaunch failed: {error}")),
    };
    for _ in 0..100 {
        if handshake.is_file() {
            let _ = fs::remove_file(&handshake);
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return rollback(
                    target,
                    &backup,
                    format!("the updated app exited during startup with {status}"),
                );
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return rollback(
                    target,
                    &backup,
                    format!("could not monitor updated app startup: {error}"),
                );
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    rollback(
        target,
        &backup,
        "the updated app did not finish starting".to_string(),
    )
}

fn rollback(target: &Path, backup: &Path, reason: String) -> Result<(), String> {
    let _ = fs::remove_file(target);
    fs::rename(backup, target)
        .map_err(|error| format!("{reason}; restoring the previous executable failed: {error}"))?;
    Err(format!("the update was rolled back because {reason}"))
}

fn wait_for_process(process_id: u32) -> Result<(), String> {
    let process = unsafe { OpenProcess(SYNCHRONIZE_PROCESS, 0, process_id) };
    if process.is_null() {
        return Ok(());
    }
    let result = unsafe { WaitForSingleObject(process, 60_000) };
    unsafe { CloseHandle(process) };
    if result == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err("the running Mujin Music for Windows did not stop in time".to_string())
    }
}

pub fn schedule_cleanup(paths: Vec<PathBuf>) {
    thread_cleanup(paths, 30, Duration::from_millis(500));
}

fn thread_cleanup(paths: Vec<PathBuf>, attempts: usize, delay: Duration) {
    std::thread::spawn(move || {
        let mut remaining = paths;
        for _ in 0..attempts {
            remaining.retain(|path| match fs::remove_file(path) {
                Ok(()) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => true,
            });
            if remaining.is_empty() {
                return;
            }
            std::thread::sleep(delay);
        }
        for path in remaining {
            tracing::warn!(path = %path.display(), "could not clean up an old update file");
        }
    });
}

pub struct StartupArguments {
    pub apply: Option<(PathBuf, u32)>,
    pub cleanup: Vec<PathBuf>,
    pub handshake: Option<PathBuf>,
}

pub fn startup_arguments() -> Result<StartupArguments, String> {
    parse_arguments(std::env::args_os().skip(1))
}

fn parse_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<StartupArguments, String> {
    let arguments: Vec<OsString> = arguments.into_iter().collect();
    let mut apply_target = None;
    let mut parent_pid = None;
    let mut cleanup = Vec::new();
    let mut handshake = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].to_string_lossy();
        match argument.as_ref() {
            "--apply-update" => {
                index += 1;
                apply_target = arguments.get(index).map(PathBuf::from);
            }
            "--parent-pid" => {
                index += 1;
                parent_pid = arguments
                    .get(index)
                    .and_then(|value| value.to_string_lossy().parse::<u32>().ok());
            }
            "--cleanup-update" | "--cleanup-backup" => {
                index += 1;
                if let Some(path) = arguments.get(index) {
                    cleanup.push(PathBuf::from(path));
                }
            }
            "--update-handshake" => {
                index += 1;
                handshake = arguments.get(index).map(PathBuf::from);
            }
            _ => {}
        }
        index += 1;
    }
    let apply = match (apply_target, parent_pid) {
        (Some(target), Some(pid)) => Some((target, pid)),
        (None, None) => None,
        _ => return Err("incomplete update installer arguments".to_string()),
    };
    Ok(StartupArguments {
        apply,
        cleanup,
        handshake,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{
        DEFAULT_RELEASES_BASE, UpdateCheck, UpdateManifest, apply_update_after_shutdown,
        default_manifest_url, parse_arguments, releases_base, validate_manifest,
        validate_manifest_from,
    };

    fn manifest() -> UpdateManifest {
        UpdateManifest {
            version: "99.0.0".to_string(),
            // The channel this build was configured for (MUJIN_RELEASES_BASE).
            url: format!("{}/download/v99.0.0/ParsonMusicServer.exe", releases_base()),
            sha256: "ab".repeat(32),
            size: 42,
        }
    }

    #[test]
    fn official_manifest_is_accepted() {
        assert!(validate_manifest(&manifest(), "https://example.test/manifest").is_ok());
    }

    #[test]
    fn manifest_urls_follow_the_configured_forge() {
        assert_eq!(
            default_manifest_url(DEFAULT_RELEASES_BASE),
            "https://github.com/mavingaw/Mujin-Music/releases/latest/download/windows-server-update.json"
        );
        assert_eq!(
            default_manifest_url("https://git.example.test/owner/repo/releases"),
            "https://git.example.test/owner/repo/releases/download/latest/windows-server-update.json"
        );
    }

    #[test]
    fn downloads_must_come_from_the_configured_release_channel() {
        let gitea = "https://git.example.test/owner/repo/releases";
        let mut candidate = manifest();
        candidate.url =
            "https://github.com/mavingaw/Mujin-Music/releases/download/v99.0.0/ParsonMusicServer.exe"
                .to_string();
        assert!(
            validate_manifest_from(&candidate, "https://m.test", DEFAULT_RELEASES_BASE, false)
                .is_ok()
        );
        assert!(validate_manifest_from(&candidate, "https://m.test", gitea, false).is_err());

        candidate.url =
            "https://git.example.test/owner/repo/releases/download/v99.0.0/ParsonMusicServer.exe"
                .to_string();
        assert!(validate_manifest_from(&candidate, "https://m.test", gitea, false).is_ok());
        assert!(
            validate_manifest_from(&candidate, "https://m.test", DEFAULT_RELEASES_BASE, false)
                .is_err()
        );
        // A manifest fetched over plain HTTP is refused unless the channel
        // was chosen explicitly with PARSON_UPDATE_MANIFEST_URL.
        assert!(validate_manifest_from(&candidate, "http://m.test", gitea, false).is_err());
        assert!(validate_manifest_from(&candidate, "http://m.test", gitea, true).is_ok());
        // A build pointed at a plain-HTTP forge never accepts an update.
        assert!(
            validate_manifest_from(
                &candidate,
                "https://m.test",
                "http://git.example.test/owner/repo/releases",
                false
            )
            .is_err()
        );

        for escape in [
            "https://git.example.test/owner/repo/releases/download/../../other/releases/download/v1.0.0/x.exe",
            "https://git.example.test/owner/repo/releases/download/v99.0.0/../x.exe",
            "https://git.example.test/owner/repo/releases/download/v99.0.0/x.exe?redirect=1",
            "https://git.example.test/owner/repo/releases/download//x.exe",
            "https://git.example.test/owner/repo/releases/download/v99.0.0/sub/x.exe",
            "https://git.example.test/owner/repo/releases/download.evil.test/v1/x.exe",
        ] {
            candidate.url = escape.to_string();
            assert!(
                validate_manifest_from(&candidate, "https://m.test", gitea, false).is_err(),
                "{escape} must be rejected"
            );
        }
    }

    #[test]
    fn malformed_hash_and_oversized_payload_are_rejected() {
        let mut candidate = manifest();
        candidate.sha256 = "not-a-hash".to_string();
        assert!(validate_manifest(&candidate, "https://example.test/manifest").is_err());
        candidate.sha256 = "ab".repeat(32);
        candidate.size = u64::MAX;
        assert!(validate_manifest(&candidate, "https://example.test/manifest").is_err());
    }

    #[test]
    fn helper_arguments_keep_paths_with_spaces_intact() {
        let parsed = parse_arguments([
            OsString::from("--apply-update"),
            OsString::from(r"C:\Program Files\Parson\ParsonMusicServer.exe"),
            OsString::from("--parent-pid"),
            OsString::from("42"),
            OsString::from("--cleanup-update"),
            OsString::from(r"C:\Temp Folder\update.exe"),
            OsString::from("--update-handshake"),
            OsString::from(r"C:\Temp Folder\ready"),
        ])
        .expect("parse update arguments");
        let (target, pid) = parsed.apply.expect("apply arguments");
        assert_eq!(
            target.to_string_lossy(),
            r"C:\Program Files\Parson\ParsonMusicServer.exe"
        );
        assert_eq!(pid, 42);
        assert_eq!(
            parsed.cleanup[0].to_string_lossy(),
            r"C:\Temp Folder\update.exe"
        );
        assert_eq!(
            parsed.handshake.expect("handshake").to_string_lossy(),
            r"C:\Temp Folder\ready"
        );
    }

    #[test]
    fn update_check_is_comparable_for_worker_tests() {
        assert_eq!(UpdateCheck::Current, UpdateCheck::Current);
    }

    #[test]
    fn failed_relaunch_restores_the_previous_executable() {
        let directory = tempfile::tempdir().expect("update rollback directory");
        let source = directory.path().join("download.exe");
        let target = directory.path().join("ParsonMusicServer.exe");
        std::fs::write(&source, b"not a Windows executable").expect("source fixture");
        std::fs::write(&target, b"previous executable").expect("target fixture");

        let error = apply_update_after_shutdown(&source, &target).expect_err("relaunch must fail");
        assert!(error.contains("rolled back"));
        assert_eq!(
            std::fs::read(&target).expect("restored target"),
            b"previous executable"
        );
        assert!(!target.with_extension("exe.update-backup").exists());
    }
}
