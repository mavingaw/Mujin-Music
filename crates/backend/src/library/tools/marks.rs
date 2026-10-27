//! What Better versions knows about individual files, layered over the
//! quality scale: copies the AI enhancer made, and lossless files the
//! spectral check caught as transcodes of lossy copies.
//!
//! The ranking of copies is fixed and deliberate:
//!
//! 1. a genuine lossless copy (a FLAC from FETCH, a CD rip);
//! 2. an AI-enhanced copy, when the admin prefers them (the default);
//! 3. the lossy original, and any "lossless" file made from a lossy one.
//!
//! An AI-enhanced copy is never graded as lossless or hi-res, whatever its
//! container says: it is labelled "AI-enhanced".

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use diesel::QueryResult;
use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};
use diesel::sqlite::SqliteConnection;

use super::quality::{Quality, QualityTier, grade};
use super::spectral::{format_khz, likely_source_kbps};
use crate::domain::AudioProperties;

/// The setting key the Better versions settings live under.
pub const SETTINGS_KEY: &str = "upgrade.settings";

#[derive(Clone, Debug, PartialEq)]
pub enum FileMark {
    /// Made by the enhancer from `source_path` with `model`, e.g.
    /// `apollo-1.0`.
    AiEnhanced { model: String, source_path: String },
    /// A lossless container whose spectrum ends like a lossy encode's.
    FakeLossless {
        cutoff_hz: Option<u32>,
        confidence: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileMarks {
    pub by_path: HashMap<String, FileMark>,
    /// Rank AI-enhanced copies above the lossy originals.
    pub prefer_enhanced: bool,
}

impl Default for FileMarks {
    fn default() -> Self {
        Self {
            by_path: HashMap::new(),
            prefer_enhanced: true,
        }
    }
}

impl FileMarks {
    pub fn get(&self, path: &str) -> Option<&FileMark> {
        self.by_path.get(path)
    }

    pub fn is_ai_enhanced(&self, path: &str) -> bool {
        matches!(self.get(path), Some(FileMark::AiEnhanced { .. }))
    }

    pub fn is_fake_lossless(&self, path: &str) -> bool {
        matches!(self.get(path), Some(FileMark::FakeLossless { .. }))
    }

    /// The enhancer model that made `path`, e.g. `apollo-1.0`.
    pub fn enhanced_model(&self, path: &str) -> Option<&str> {
        match self.get(path) {
            Some(FileMark::AiEnhanced { model, .. }) => Some(model),
            _ => None,
        }
    }
}

static CURRENT: OnceLock<RwLock<Arc<FileMarks>>> = OnceLock::new();

fn slot() -> &'static RwLock<Arc<FileMarks>> {
    CURRENT.get_or_init(|| RwLock::new(Arc::new(FileMarks::default())))
}

/// The marks the catalog was last built with.
pub fn current() -> Arc<FileMarks> {
    slot().read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Replaces the marks (done whenever the catalog layers are loaded).
pub fn install(marks: FileMarks) -> Arc<FileMarks> {
    let marks = Arc::new(marks);
    *slot().write().unwrap_or_else(|p| p.into_inner()) = marks.clone();
    marks
}

/// "Apollo" from `apollo-1.0`.
pub fn model_label(model: &str) -> String {
    let name = model.split('-').next().unwrap_or(model);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "AI".into(),
    }
}

#[derive(QueryableByName)]
struct EnhancedRow {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Text)]
    source_path: String,
    #[diesel(sql_type = Text)]
    model: String,
}

#[derive(QueryableByName)]
struct FakeRow {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Nullable<diesel::sql_types::Integer>)]
    cutoff_hz: Option<i32>,
    #[diesel(sql_type = Double)]
    confidence: f64,
}

#[derive(QueryableByName)]
struct SettingRow {
    #[diesel(sql_type = Text)]
    value: String,
}

/// Whether AI-enhanced copies outrank lossy originals (default on).
pub fn prefer_enhanced(connection: &mut SqliteConnection) -> bool {
    diesel::sql_query("SELECT value_json AS value FROM server_setting WHERE key = ?")
        .bind::<Text, _>(SETTINGS_KEY)
        .load::<SettingRow>(connection)
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|row| serde_json::from_str::<serde_json::Value>(&row.value).ok())
        .and_then(|value| value.pointer("/enhancer/prefer_enhanced")?.as_bool())
        .unwrap_or(true)
}

/// Reads every mark from the database.
pub fn load(connection: &mut SqliteConnection) -> QueryResult<FileMarks> {
    let mut by_path = HashMap::new();
    for row in diesel::sql_query(
        "SELECT path, cutoff_hz, confidence FROM upgrade_spectral_check
         WHERE verdict = 'transcode' AND confidence >= 0.6",
    )
    .load::<FakeRow>(connection)?
    {
        by_path.insert(
            row.path,
            FileMark::FakeLossless {
                cutoff_hz: row.cutoff_hz.and_then(|hz| u32::try_from(hz).ok()),
                confidence: row.confidence as f32,
            },
        );
    }
    for row in diesel::sql_query("SELECT path, source_path, model FROM upgrade_enhanced_file")
        .load::<EnhancedRow>(connection)?
    {
        by_path.insert(
            row.path,
            FileMark::AiEnhanced {
                model: row.model,
                source_path: row.source_path,
            },
        );
    }
    Ok(FileMarks {
        by_path,
        prefer_enhanced: prefer_enhanced(connection),
    })
}

/// Grades a file with its marks applied.
pub fn grade_marked(
    audio: Option<&AudioProperties>,
    extension: &str,
    mark: Option<&FileMark>,
) -> Quality {
    let base = grade(audio, extension);
    match mark {
        Some(FileMark::AiEnhanced { model, .. }) => Quality {
            tier: QualityTier::AiEnhanced,
            score: QualityTier::AiEnhanced.rank() * 10_000_000,
            label: format!("AI-enhanced ({})", model_label(model)),
            format: base.format,
        },
        Some(FileMark::FakeLossless { cutoff_hz, .. })
            if matches!(base.tier, QualityTier::HiRes | QualityTier::Lossless) =>
        {
            let kbps = cutoff_hz.map_or(128, likely_source_kbps);
            Quality {
                tier: QualityTier::Lossy,
                // Just below a real lossy copy of the same bitrate.
                score: (QualityTier::Lossy.rank() * 10_000_000 + u64::from(kbps) * 100)
                    .saturating_sub(1),
                label: match cutoff_hz {
                    Some(hz) => format!("{} · from lossy ({})", base.format, format_khz(*hz)),
                    None => format!("{} · from lossy", base.format),
                },
                format: base.format,
            }
        }
        _ => base,
    }
}

/// Where a copy stands when choosing which one plays: higher is better.
/// The class (3 genuine lossless, 2 AI-enhanced when preferred, 1 lossy or
/// fake lossless, 0 AI-enhanced when not preferred) always wins; within a
/// class a real lossy original beats a fake lossless file, whatever
/// bitrate the fake came from; quality breaks the remaining ties.
pub fn copy_rank(
    audio: Option<&AudioProperties>,
    extension: &str,
    mark: Option<&FileMark>,
    prefer_enhanced: bool,
) -> (u8, u8, u64) {
    let quality = grade_marked(audio, extension, mark);
    let class = match quality.tier {
        QualityTier::AiEnhanced if prefer_enhanced => 2,
        QualityTier::AiEnhanced => 0,
        QualityTier::Dsd | QualityTier::HiRes | QualityTier::Lossless => 3,
        _ => 1,
    };
    let honest = u8::from(!matches!(mark, Some(FileMark::FakeLossless { .. })));
    (class, honest, quality.score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(codec: &str, rate: u32, bits: Option<u8>, kbps: u32) -> AudioProperties {
        AudioProperties {
            codec: Some(codec.into()),
            sample_rate: Some(rate),
            bit_depth: bits,
            channels: Some(2),
            bitrate_kbps: Some(kbps),
            lossless: crate::domain::codec_is_lossless(codec),
            ..Default::default()
        }
    }

    #[test]
    fn lossless_beats_ai_enhanced_beats_lossy_and_fakes() {
        let flac = audio("flac", 44_100, Some(16), 900);
        let enhanced_flac = audio("flac", 44_100, Some(24), 1_400);
        let mp3 = audio("mp3", 44_100, None, 320);
        let enhanced = FileMark::AiEnhanced {
            model: "apollo-1.0".into(),
            source_path: "/m/a.mp3".into(),
        };
        let fake = FileMark::FakeLossless {
            cutoff_hz: Some(16_000),
            confidence: 0.9,
        };
        let lossless = copy_rank(Some(&flac), "flac", None, true);
        let ai = copy_rank(Some(&enhanced_flac), "flac", Some(&enhanced), true);
        let lossy = copy_rank(Some(&mp3), "mp3", None, true);
        let faked = copy_rank(Some(&flac), "flac", Some(&fake), true);
        // Even an MP3 128 original beats a fake made from a 320.
        let low = audio("mp3", 44_100, None, 128);
        let fake_320 = FileMark::FakeLossless {
            cutoff_hz: Some(20_000),
            confidence: 0.7,
        };
        assert!(
            copy_rank(Some(&low), "mp3", None, true)
                > copy_rank(Some(&flac), "flac", Some(&fake_320), true)
        );
        assert!(lossless > ai, "a real lossless copy outranks AI");
        assert!(ai > lossy, "AI-enhanced outranks lossy when preferred");
        assert!(
            lossy > faked,
            "the MP3 320 original outranks a fake from 128"
        );
        // Not preferred: the AI copy drops below the original.
        let ai_off = copy_rank(Some(&enhanced_flac), "flac", Some(&enhanced), false);
        assert!(lossy > ai_off);
    }

    #[test]
    fn ai_enhanced_is_never_labelled_lossless_or_hi_res() {
        let enhanced = FileMark::AiEnhanced {
            model: "apollo-1.0".into(),
            source_path: "/m/a.mp3".into(),
        };
        let quality = grade_marked(
            Some(&audio("flac", 44_100, Some(24), 1_400)),
            "flac",
            Some(&enhanced),
        );
        assert_eq!(quality.tier, QualityTier::AiEnhanced);
        assert_eq!(quality.label, "AI-enhanced (Apollo)");
        assert!(!quality.label.to_lowercase().contains("lossless"));
        assert!(!quality.label.to_lowercase().contains("hi-res"));
        assert_eq!(QualityTier::AiEnhanced.label(), "AI-enhanced");
    }

    #[test]
    fn fake_lossless_is_graded_as_the_lossy_copy_it_came_from() {
        let fake = FileMark::FakeLossless {
            cutoff_hz: Some(16_000),
            confidence: 0.9,
        };
        let quality = grade_marked(
            Some(&audio("flac", 44_100, Some(16), 900)),
            "flac",
            Some(&fake),
        );
        assert_eq!(quality.tier, QualityTier::Lossy);
        assert_eq!(quality.label, "FLAC · from lossy (16 kHz)");
    }

    #[test]
    fn marks_load_from_the_database_with_the_preference() {
        let pool = crate::persistence::connection::migrated_memory_pool();
        let mut connection = pool.get().unwrap();
        diesel::sql_query(
            "INSERT INTO upgrade_spectral_check (path, file_version, verdict, cutoff_hz, confidence)
             VALUES ('/m/fake.flac', '1:1', 'transcode', 16000, 0.9),
                    ('/m/unsure.flac', '1:1', 'transcode', 19500, 0.4),
                    ('/m/real.flac', '1:1', 'genuine', NULL, 0.9)",
        )
        .execute(&mut connection)
        .unwrap();
        diesel::sql_query(
            "INSERT INTO upgrade_enhanced_file (path, source_path, model)
             VALUES ('/m/Enhanced/a.flac', '/m/a.mp3', 'apollo-1.0')",
        )
        .execute(&mut connection)
        .unwrap();
        let marks = load(&mut connection).unwrap();
        assert!(marks.is_fake_lossless("/m/fake.flac"));
        assert!(!marks.is_fake_lossless("/m/unsure.flac"), "low confidence");
        assert!(marks.get("/m/real.flac").is_none());
        assert_eq!(
            marks.enhanced_model("/m/Enhanced/a.flac"),
            Some("apollo-1.0")
        );
        assert!(marks.prefer_enhanced, "on by default");
        diesel::sql_query(
            "INSERT INTO server_setting (key, value_json) VALUES (?, '{\"enhancer\":{\"prefer_enhanced\":false}}')",
        )
        .bind::<Text, _>(SETTINGS_KEY)
        .execute(&mut connection)
        .unwrap();
        assert!(!load(&mut connection).unwrap().prefer_enhanced);
        assert_eq!(model_label("apollo-1.0"), "Apollo");
    }
}
