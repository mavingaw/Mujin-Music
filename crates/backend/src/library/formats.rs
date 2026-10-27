//! The audio file formats Parson recognises, in one place.
//!
//! Discovery, the indexer, setup suggestions, release grouping and the media
//! endpoints all read this table, so adding a format here makes it visible
//! everywhere at once.

/// How a file of this format is read during indexing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseRoute {
    /// A Parson fast path or lofty reads tags and stream headers in-process.
    Native,
    /// No in-process reader exists; `ffprobe` reads tags and properties.
    Ffprobe,
}

/// Static facts about one file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    /// Lower-case file extension without the dot.
    pub extension: &'static str,
    /// Human name shown in the UI, e.g. `Monkey's Audio`.
    pub name: &'static str,
    /// MIME type served for the original bytes.
    pub content_type: &'static str,
    /// Whether every codec this container normally carries is lossless.
    /// `None` when the container holds both kinds (Matroska, WMA, MP4).
    pub lossless: Option<bool>,
    pub route: ParseRoute,
}

const fn native(
    extension: &'static str,
    name: &'static str,
    content_type: &'static str,
    lossless: Option<bool>,
) -> FormatInfo {
    FormatInfo {
        extension,
        name,
        content_type,
        lossless,
        route: ParseRoute::Native,
    }
}

const fn probed(
    extension: &'static str,
    name: &'static str,
    content_type: &'static str,
    lossless: Option<bool>,
) -> FormatInfo {
    FormatInfo {
        extension,
        name,
        content_type,
        lossless,
        route: ParseRoute::Ffprobe,
    }
}

/// Every audio format the library indexes.
pub const FORMATS: &[FormatInfo] = &[
    native("flac", "FLAC", "audio/flac", Some(true)),
    native("m4a", "MPEG-4 Audio", "audio/mp4", None),
    native("alac", "Apple Lossless", "audio/mp4", Some(true)),
    native("wav", "WAV", "audio/wav", None),
    native("aiff", "AIFF", "audio/aiff", Some(true)),
    native("aif", "AIFF", "audio/aiff", Some(true)),
    native("aifc", "AIFF-C", "audio/aiff", None),
    native("wv", "WavPack", "audio/x-wavpack", None),
    native("ape", "Monkey's Audio", "audio/x-ape", Some(true)),
    probed("tak", "TAK", "audio/x-tak", Some(true)),
    probed("tta", "True Audio", "audio/x-tta", Some(true)),
    native("dsf", "DSD Stream File", "audio/x-dsf", Some(true)),
    native("dff", "DSDIFF", "audio/x-dff", Some(true)),
    native("mp3", "MP3", "audio/mpeg", Some(false)),
    native("aac", "AAC", "audio/aac", Some(false)),
    native("ogg", "Ogg", "audio/ogg", None),
    native("oga", "Ogg", "audio/ogg", None),
    native("opus", "Opus", "audio/ogg", Some(false)),
    native("mpc", "Musepack", "audio/x-musepack", Some(false)),
    probed("wma", "Windows Media Audio", "audio/x-ms-wma", None),
    probed("mka", "Matroska Audio", "audio/x-matroska", None),
    probed("caf", "Core Audio Format", "audio/x-caf", None),
    probed("w64", "Sony Wave64", "audio/x-w64", None),
];

/// Extensions of every indexed audio format, lower case.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "m4a", "alac", "wav", "aiff", "aif", "aifc", "wv", "ape", "tak", "tta", "dsf", "dff",
    "mp3", "aac", "ogg", "oga", "opus", "mpc", "wma", "mka", "caf", "w64",
];

/// Looks up a format by extension (case-insensitive, without the dot).
pub fn format_for_extension(extension: &str) -> Option<&'static FormatInfo> {
    FORMATS
        .iter()
        .find(|format| format.extension.eq_ignore_ascii_case(extension))
}

/// Whether the extension belongs to an indexed audio format.
pub fn is_audio_extension(extension: &str) -> bool {
    format_for_extension(extension).is_some()
}

/// Whether the path names an indexed audio file.
pub fn is_audio_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_audio_extension)
}

/// MIME type for the original bytes of a file.
pub fn content_type_for_path(path: &std::path::Path) -> &'static str {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(format_for_extension)
        .map(|format| format.content_type)
        .unwrap_or("application/octet-stream")
}

/// DSD rate name (`DSD64`, `DSD128`, ...) for a 1-bit sample rate in Hz.
pub fn dsd_rate_name(sample_rate: u32) -> Option<String> {
    if sample_rate < 2_000_000 {
        return None;
    }
    let base = if sample_rate.is_multiple_of(44_100) {
        44_100
    } else if sample_rate.is_multiple_of(48_000) {
        48_000
    } else {
        44_100
    };
    let multiple = (f64::from(sample_rate) / f64::from(base)).round() as u32;
    Some(format!("DSD{multiple}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_list_and_table_agree() {
        assert_eq!(AUDIO_EXTENSIONS.len(), FORMATS.len());
        for format in FORMATS {
            assert!(
                AUDIO_EXTENSIONS.contains(&format.extension),
                "{} missing from AUDIO_EXTENSIONS",
                format.extension
            );
        }
    }

    #[test]
    fn collector_formats_are_recognised_case_insensitively() {
        for extension in [
            "FLAC", "alac", "wav", "AIFF", "aif", "wv", "ape", "tak", "dsf", "DFF", "opus", "ogg",
            "mp3", "m4a", "aac", "wma", "mka", "tta", "mpc",
        ] {
            assert!(is_audio_extension(extension), "{extension}");
        }
        assert!(!is_audio_extension("jpg"));
        assert!(!is_audio_extension("cue"));
        assert_eq!(
            content_type_for_path(std::path::Path::new("/a/b.FLAC")),
            "audio/flac"
        );
        assert_eq!(
            format_for_extension("tak").map(|format| format.route),
            Some(ParseRoute::Ffprobe)
        );
    }

    #[test]
    fn dsd_rates_are_named_by_their_multiple_of_44_1k() {
        assert_eq!(dsd_rate_name(2_822_400).as_deref(), Some("DSD64"));
        assert_eq!(dsd_rate_name(5_644_800).as_deref(), Some("DSD128"));
        assert_eq!(dsd_rate_name(11_289_600).as_deref(), Some("DSD256"));
        assert_eq!(dsd_rate_name(3_072_000).as_deref(), Some("DSD64"));
        assert_eq!(dsd_rate_name(96_000), None);
    }
}
