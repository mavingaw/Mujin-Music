//! Skipped files: every file in a music folder that looks like it should
//! have become music (or artwork, or a playlist) and did not, with the exact
//! reason and a way to fix it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::library::discovery::{AuditEntry, audit_walk};
use crate::library::formats::AUDIO_EXTENSIONS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The file reads, but its stream headers do not.
    Corrupt,
    /// Imported with names from the file path because its tags are broken.
    TagsUnreadable,
    /// The container holds no audio frames Parson can measure.
    NoAudio,
    /// A zero-byte file.
    EmptyFile,
    /// The operating system refused to open the file.
    IoFailure,
    /// A folder the scan could not list.
    UnreadableFolder,
    /// A music file that arrived after the last scan.
    NotYetImported,
    /// Audio in a format Parson does not decode.
    UnsupportedFormat,
    /// Purchased audio locked with DRM.
    DrmProtected,
    /// A video file in a music folder.
    VideoFile,
    /// A disc image (SACD ISO, CUE + single file album image).
    DiscImage,
    /// A playlist file, which is imported on request rather than scanned.
    PlaylistFile,
    /// Artwork that cannot be decoded or is in a format covers do not use.
    UnreadableImage,
}

impl SkipReason {
    pub const ALL: [Self; 13] = [
        Self::Corrupt,
        Self::TagsUnreadable,
        Self::NoAudio,
        Self::EmptyFile,
        Self::IoFailure,
        Self::UnreadableFolder,
        Self::NotYetImported,
        Self::UnsupportedFormat,
        Self::DrmProtected,
        Self::VideoFile,
        Self::DiscImage,
        Self::PlaylistFile,
        Self::UnreadableImage,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Corrupt => "corrupt",
            Self::TagsUnreadable => "tags_unreadable",
            Self::NoAudio => "no_audio",
            Self::EmptyFile => "empty_file",
            Self::IoFailure => "io_failure",
            Self::UnreadableFolder => "unreadable_folder",
            Self::NotYetImported => "not_yet_imported",
            Self::UnsupportedFormat => "unsupported_format",
            Self::DrmProtected => "drm_protected",
            Self::VideoFile => "video_file",
            Self::DiscImage => "disc_image",
            Self::PlaylistFile => "playlist_file",
            Self::UnreadableImage => "unreadable_image",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.as_str() == value)
    }

    /// Short label, as in Roon's list ("File is corrupt").
    pub fn label(self) -> &'static str {
        match self {
            Self::Corrupt => "Audio stream unreadable",
            Self::TagsUnreadable => "Tags unreadable",
            Self::NoAudio => "No audio found",
            Self::EmptyFile => "Empty file",
            Self::IoFailure => "Can't open the file",
            Self::UnreadableFolder => "Folder can't be read",
            Self::NotYetImported => "Waiting for the next scan",
            Self::UnsupportedFormat => "Format not supported",
            Self::DrmProtected => "Copy-protected (DRM)",
            Self::VideoFile => "Video file",
            Self::DiscImage => "Disc image",
            Self::PlaylistFile => "Playlist file",
            Self::UnreadableImage => "Artwork unreadable",
        }
    }

    /// Whether the file is in the library anyway (with reduced metadata).
    pub fn imported(self) -> bool {
        matches!(self, Self::TagsUnreadable)
    }
}

/// How to fix a skipped file. `action` names a button the app offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Fix {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "wmv", "flv", "mpg", "mpeg", "vob", "ts", "m2ts",
    "3gp",
];
const UNSUPPORTED_AUDIO: &[&str] = &[
    "mp2", "mpa", "ac3", "eac3", "dts", "dtshd", "thd", "mlp", "amr", "awb", "ra", "rm", "ram",
    "shn", "ofr", "ofs", "spx", "au", "snd", "voc", "mid", "midi", "kar", "mod", "xm", "it", "s3m",
    "m4b", "aa", "aax", "weba", "qcp", "gsm", "8svx", "sd2", "la", "pac", "vqf",
];
const PLAYLIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls", "xspf", "wpl", "asx"];
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];
const UNUSED_IMAGE_EXTENSIONS: &[&str] = &["bmp", "gif", "tif", "tiff", "heic", "heif", "avif"];
const COVER_STEMS: &[&str] = &["cover", "folder", "front", "album", "albumart", "artwork"];

/// Reason for a file the scanner does not import, judged by its name alone.
/// `None` for files that are not media (text, logs, cue sheets of track
/// rips, ...) and for audio formats the scanner imports.
pub fn classify_extension(path: &Path, extension: &str) -> Option<(SkipReason, String)> {
    let extension = extension.to_ascii_lowercase();
    let extension = extension.as_str();
    if AUDIO_EXTENSIONS.contains(&extension) || IMAGE_EXTENSIONS.contains(&extension) {
        return None;
    }
    if extension == "m4p" {
        return Some((
            SkipReason::DrmProtected,
            "Protected AAC from the iTunes Store (FairPlay DRM). Its audio cannot be decoded \
             by any player other than Apple's."
                .into(),
        ));
    }
    if VIDEO_EXTENSIONS.contains(&extension) {
        return Some((
            SkipReason::VideoFile,
            format!(
                "{} video. Mujin Music imports audio files; a music video or concert film is left out.",
                extension.to_ascii_uppercase()
            ),
        ));
    }
    if extension == "iso" {
        return Some((
            SkipReason::DiscImage,
            "Disc image (often an SACD ISO). Mujin Music reads DSF and DFF files, not ISO images."
                .into(),
        ));
    }
    if UNSUPPORTED_AUDIO.contains(&extension) {
        let detail = match extension {
            "m4b" => "Audiobook in MPEG-4 (M4B). Audiobooks are kept out of the music library.",
            "aa" | "aax" => "Audible audiobook, protected by DRM.",
            "mid" | "midi" | "kar" => "MIDI score: notes, not recorded audio.",
            "mod" | "xm" | "it" | "s3m" => {
                "Tracker module: instructions for a synthesiser, not recorded audio."
            }
            "ac3" | "eac3" | "dts" | "dtshd" | "thd" | "mlp" => {
                "Surround soundtrack stream without a music container."
            }
            "shn" | "ofr" | "ofs" | "la" | "pac" => {
                "Legacy lossless codec that Mujin Music does not decode."
            }
            _ => "Audio format Mujin Music does not decode.",
        };
        return Some((SkipReason::UnsupportedFormat, detail.into()));
    }
    if PLAYLIST_EXTENSIONS.contains(&extension) {
        return Some((
            SkipReason::PlaylistFile,
            "Playlists are imported on request, so a scan never changes your playlists.".into(),
        ));
    }
    if UNUSED_IMAGE_EXTENSIONS.contains(&extension) {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if COVER_STEMS.iter().any(|cover| stem.starts_with(cover)) {
            return Some((
                SkipReason::UnreadableImage,
                format!(
                    "Cover art in {} is not used; covers are read from JPEG, PNG or WebP.",
                    extension.to_ascii_uppercase()
                ),
            ));
        }
    }
    None
}

fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// What the user can do about a skipped file.
pub fn fix_for(reason: SkipReason, extension: &str, path: &str) -> Fix {
    let stem_target = |new_extension: &str| {
        let target = Path::new(path).with_extension(new_extension);
        target.to_string_lossy().into_owned()
    };
    match reason {
        SkipReason::Corrupt => Fix {
            summary: "Replace the file with a fresh copy from its source (CD rip, purchase or \
                      backup), then read it again. If other players also fail, the file is \
                      damaged."
                .into(),
            action: Some("rescan"),
            command: Some(format!(
                "ffmpeg -v error -i {} -f null -",
                shell_quote(path)
            )),
        },
        SkipReason::TagsUnreadable => Fix {
            summary: "The track is in your library under its file name. Fix the tags in a tag \
                      editor and read it again, or edit its details in Mujin Music."
                .into(),
            action: Some("edit"),
            command: None,
        },
        SkipReason::NoAudio => Fix {
            summary:
                "The file has a music extension but no audio Mujin Music can measure. Check it \
                      plays elsewhere, then read it again."
                    .into(),
            action: Some("rescan"),
            command: None,
        },
        SkipReason::EmptyFile => Fix {
            summary: "The file is 0 bytes, usually an interrupted copy or download. Copy it \
                      again from its source."
                .into(),
            action: Some("rescan"),
            command: None,
        },
        SkipReason::IoFailure => Fix {
            summary: "The server can't open this file. Check that the account running Mujin Music \
                      may read it, and that the drive or network share is healthy."
                .into(),
            action: Some("rescan"),
            command: Some(format!("chmod a+r {}", shell_quote(path))),
        },
        SkipReason::UnreadableFolder => Fix {
            summary:
                "Mujin Music can't list this folder, so nothing inside it is in your library. \
                      Check its permissions and that the drive is connected."
                    .into(),
            action: Some("rescan"),
            command: Some(format!("chmod a+rx {}", shell_quote(path))),
        },
        SkipReason::NotYetImported => Fix {
            summary: "New music is picked up automatically. Scan now to add it straight away."
                .into(),
            action: Some("rescan"),
            command: None,
        },
        SkipReason::DrmProtected => Fix {
            summary: "Download the track again from your Apple purchases: iTunes Plus files \
                      (.m4a) are DRM-free and import directly."
                .into(),
            action: None,
            command: None,
        },
        SkipReason::VideoFile => Fix {
            summary: "To add the soundtrack, extract the audio without re-encoding into a \
                      separate file; the video stays untouched."
                .into(),
            action: None,
            command: Some(format!(
                "ffmpeg -i {} -vn -c:a copy {}",
                shell_quote(path),
                shell_quote(&stem_target("mka"))
            )),
        },
        SkipReason::DiscImage => Fix {
            summary: if extension.eq_ignore_ascii_case("cue") {
                "This album is one long audio file with a CUE sheet, so it plays as one track. \
                 Split it into tracks losslessly (for example with shnsplit or XLD)."
                    .into()
            } else {
                "Extract the DSD tracks to DSF files (for example with sacd_extract), keeping \
                 the full DSD resolution."
                    .into()
            },
            action: None,
            command: if extension.eq_ignore_ascii_case("cue") {
                None
            } else {
                Some(format!("sacd_extract -s -c -i {}", shell_quote(path)))
            },
        },
        SkipReason::PlaylistFile => Fix {
            summary: "Import it to make a Mujin Music playlist from the tracks it lists.".into(),
            action: Some("import_playlist"),
            command: None,
        },
        SkipReason::UnreadableImage => Fix {
            summary: "Save the cover as JPEG or PNG (at least 1000 pixels square looks best)."
                .into(),
            action: None,
            command: Some(format!(
                "ffmpeg -i {} {}",
                shell_quote(path),
                shell_quote(&stem_target("jpg"))
            )),
        },
        SkipReason::UnsupportedFormat => Fix {
            summary: match extension {
                "shn" | "ofr" | "ofs" | "la" | "pac" => {
                    "Convert it losslessly to FLAC; every bit of the original is kept.".into()
                }
                "m4b" => "If this is music rather than an audiobook, rename it to .m4a.".into(),
                _ => "Convert it to FLAC (lossless) or keep the original elsewhere.".into(),
            },
            action: None,
            command: matches!(
                extension,
                "shn" | "ofr" | "ofs" | "la" | "pac" | "ac3" | "dts" | "mp2"
            )
            .then(|| {
                format!(
                    "ffmpeg -i {} -c:a flac {}",
                    shell_quote(path),
                    shell_quote(&stem_target("flac"))
                )
            }),
        },
    }
}

/// One finding of the audit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedRecord {
    pub path: String,
    pub root_path: String,
    pub reason: SkipReason,
    pub detail: String,
    pub extension: String,
    pub size_bytes: i64,
    pub modified_at_ns: i64,
}

/// What the catalog knows about an imported file, for the audit.
#[derive(Clone, Debug)]
pub struct ImportedFile {
    pub error: Option<String>,
    pub duration_seconds: f64,
}

fn database_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Audits one music folder. `imported` holds every file the catalog has,
/// by database path. `should_stop` is polled between files.
pub fn audit_root(
    root: &Path,
    imported: &HashMap<String, ImportedFile>,
    should_stop: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize),
) -> Vec<SkippedRecord> {
    let root_path = database_path(root);
    let mut records = Vec::new();
    let mut audio_per_directory: HashMap<String, usize> = HashMap::new();
    let mut cue_sheets: Vec<(String, String, i64, i64)> = Vec::new();
    let mut seen = 0usize;
    audit_walk(root, &mut |entry| {
        if should_stop() {
            return false;
        }
        match entry {
            AuditEntry::Unreadable { path, error } => records.push(SkippedRecord {
                path: database_path(&path),
                root_path: root_path.clone(),
                reason: SkipReason::UnreadableFolder,
                detail: error,
                extension: String::new(),
                size_bytes: 0,
                modified_at_ns: 0,
            }),
            AuditEntry::File {
                path,
                extension,
                size_bytes,
                modified_at_ns,
            } => {
                seen += 1;
                if seen.is_multiple_of(500) {
                    progress(seen);
                }
                let key = database_path(&path);
                let directory = path.parent().map(database_path).unwrap_or_default();
                if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
                    *audio_per_directory.entry(directory).or_default() += 1;
                    if let Some(record) = audit_audio_file(
                        &path,
                        &key,
                        &root_path,
                        &extension,
                        size_bytes,
                        modified_at_ns,
                        imported,
                    ) {
                        records.push(record);
                    }
                    return true;
                }
                if extension == "cue" {
                    cue_sheets.push((key, directory, size_bytes, modified_at_ns));
                    return true;
                }
                if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                    if size_bytes == 0 || image::image_dimensions(&path).is_err() {
                        records.push(SkippedRecord {
                            path: key,
                            root_path: root_path.clone(),
                            reason: SkipReason::UnreadableImage,
                            detail: if size_bytes == 0 {
                                "The image file is empty.".into()
                            } else {
                                "The image headers could not be read, so it can't be used as cover art."
                                    .into()
                            },
                            extension,
                            size_bytes,
                            modified_at_ns,
                        });
                    }
                    return true;
                }
                if let Some((reason, detail)) = classify_extension(&path, &extension) {
                    records.push(SkippedRecord {
                        path: key,
                        root_path: root_path.clone(),
                        reason,
                        detail,
                        extension,
                        size_bytes,
                        modified_at_ns,
                    });
                }
            }
        }
        true
    });
    // A cue sheet beside exactly one audio file describes an album image.
    for (path, directory, size_bytes, modified_at_ns) in cue_sheets {
        if audio_per_directory.get(&directory).copied() == Some(1) {
            records.push(SkippedRecord {
                path,
                root_path: root_path.clone(),
                reason: SkipReason::DiscImage,
                detail: "CUE sheet for an album stored as one audio file; the album plays as a \
                         single long track."
                    .into(),
                extension: "cue".into(),
                size_bytes,
                modified_at_ns,
            });
        }
    }
    progress(seen);
    records
}

fn audit_audio_file(
    path: &Path,
    key: &str,
    root_path: &str,
    extension: &str,
    size_bytes: i64,
    modified_at_ns: i64,
    imported: &HashMap<String, ImportedFile>,
) -> Option<SkippedRecord> {
    let record = |reason: SkipReason, detail: String| SkippedRecord {
        path: key.to_string(),
        root_path: root_path.to_string(),
        reason,
        detail,
        extension: extension.to_string(),
        size_bytes,
        modified_at_ns,
    };
    if size_bytes == 0 {
        return Some(record(
            SkipReason::EmptyFile,
            "The file is 0 bytes long.".into(),
        ));
    }
    match imported.get(key) {
        Some(file) => {
            let has_audio = file.duration_seconds.is_finite() && file.duration_seconds > 0.0;
            match (&file.error, has_audio) {
                (Some(error), false) => Some(record(
                    SkipReason::Corrupt,
                    format!("The audio reader stopped: {error}."),
                )),
                (Some(error), true) => Some(record(
                    SkipReason::TagsUnreadable,
                    format!("The tag reader stopped: {error}."),
                )),
                (None, false) => Some(record(
                    SkipReason::NoAudio,
                    "The stream headers report no playable length.".into(),
                )),
                (None, true) => None,
            }
        }
        None => match std::fs::File::open(path) {
            Ok(_) => Some(record(
                SkipReason::NotYetImported,
                "This file appeared after the last library scan.".into(),
            )),
            Err(error) => Some(record(SkipReason::IoFailure, error.to_string())),
        },
    }
}

/// Paths of audit findings whose reason no longer applies, for pruning.
pub fn stale_paths(previous: &[String], current: &[SkippedRecord]) -> Vec<String> {
    let current: HashSet<&str> = current.iter().map(|record| record.path.as_str()).collect();
    previous
        .iter()
        .filter(|path| !current.contains(path.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_get_exact_reasons_and_ordinary_files_are_ignored() {
        let reason = |name: &str| {
            let path = Path::new(name);
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            classify_extension(path, extension).map(|(reason, _)| reason)
        };
        assert_eq!(reason("/m/a.m4p"), Some(SkipReason::DrmProtected));
        assert_eq!(reason("/m/live.mkv"), Some(SkipReason::VideoFile));
        assert_eq!(reason("/m/a.shn"), Some(SkipReason::UnsupportedFormat));
        assert_eq!(reason("/m/sacd.iso"), Some(SkipReason::DiscImage));
        assert_eq!(reason("/m/mix.m3u8"), Some(SkipReason::PlaylistFile));
        assert_eq!(reason("/m/cover.bmp"), Some(SkipReason::UnreadableImage));
        assert_eq!(reason("/m/scan-04.bmp"), None);
        assert_eq!(reason("/m/a.flac"), None);
        assert_eq!(reason("/m/rip.log"), None);
        assert_eq!(reason("/m/notes.txt"), None);
    }

    #[test]
    fn every_reason_round_trips_and_has_a_fix() {
        for reason in SkipReason::ALL {
            assert_eq!(SkipReason::parse(reason.as_str()), Some(reason));
            assert!(!fix_for(reason, "x", "/m/a.x").summary.is_empty());
        }
        let video = fix_for(SkipReason::VideoFile, "mkv", "/m/It's Live.mkv");
        assert_eq!(
            video.command.as_deref(),
            Some("ffmpeg -i '/m/It'\\''s Live.mkv' -vn -c:a copy '/m/It'\\''s Live.mka'")
        );
        assert_eq!(
            fix_for(SkipReason::PlaylistFile, "m3u", "/m/a.m3u").action,
            Some("import_playlist")
        );
    }

    #[test]
    fn the_audit_reads_a_folder_like_the_scanner() {
        let root = std::env::temp_dir().join(format!("parson-skipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Album")).unwrap();
        std::fs::create_dir_all(root.join("Image")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("Album/01.flac"), b"fLaC....").unwrap();
        std::fs::write(root.join("Album/02.flac"), b"fLaC....").unwrap();
        std::fs::write(root.join("Album/03.flac"), b"").unwrap();
        std::fs::write(root.join("Album/rip.cue"), b"FILE").unwrap();
        std::fs::write(root.join("Album/cover.jpg"), b"not a jpeg").unwrap();
        std::fs::write(root.join("Album/video.mp4"), b"....").unwrap();
        std::fs::write(root.join("Album/notes.txt"), b"hello").unwrap();
        std::fs::write(root.join("Image/album.flac"), b"fLaC....").unwrap();
        std::fs::write(root.join("Image/album.cue"), b"FILE").unwrap();
        std::fs::write(root.join(".hidden/secret.mp4"), b"....").unwrap();
        let path_of = |relative: &str| database_path(&root.join(relative));
        let imported = HashMap::from([
            (
                path_of("Album/01.flac"),
                ImportedFile {
                    error: None,
                    duration_seconds: 200.0,
                },
            ),
            (
                path_of("Image/album.flac"),
                ImportedFile {
                    error: Some("broken Vorbis comment".into()),
                    duration_seconds: 2400.0,
                },
            ),
        ]);
        let mut records = audit_root(&root, &imported, &|| false, &mut |_| {});
        records.sort_by(|a, b| a.path.cmp(&b.path));
        let found: Vec<(String, SkipReason)> = records
            .iter()
            .map(|record| {
                (
                    record
                        .path
                        .trim_start_matches(&database_path(&root))
                        .to_string(),
                    record.reason,
                )
            })
            .collect();
        assert_eq!(
            found,
            vec![
                ("/Album/02.flac".to_string(), SkipReason::NotYetImported),
                ("/Album/03.flac".to_string(), SkipReason::EmptyFile),
                ("/Album/cover.jpg".to_string(), SkipReason::UnreadableImage),
                ("/Album/video.mp4".to_string(), SkipReason::VideoFile),
                ("/Image/album.cue".to_string(), SkipReason::DiscImage),
                ("/Image/album.flac".to_string(), SkipReason::TagsUnreadable),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
