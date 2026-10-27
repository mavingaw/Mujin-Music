//! Volume leveling (Roon's Off/Auto/Track/Album): EBU R128 loudness of each
//! track, measured once with ffmpeg and stored, and the gain a zone applies
//! to reach its target loudness without clipping.
//!
//! Sonos zones get the gain applied by the server in the stream it serves;
//! Parson apps apply it in their own player.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

use super::model::LevelingMode;
use crate::library::state::LibraryCache;
use crate::persistence::connection::DbPool;

/// Gain is never pushed past this, whatever the target says.
pub const MAX_BOOST_DB: f32 = 12.0;
pub const MAX_CUT_DB: f32 = -24.0;
/// Leveled audio keeps its true peak at or below this.
pub const PEAK_CEILING_DBTP: f32 = -1.0;
const MEASURE_TIMEOUT: Duration = Duration::from_secs(180);
const SILENCE_LUFS: f32 = -70.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Loudness {
    /// EBU R128 integrated loudness.
    pub integrated_lufs: f32,
    /// True peak in dBTP.
    pub true_peak_db: f32,
}

/// Leveling resolved for one queued track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelBy {
    Track,
    Album,
}

impl LevelBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "track" => Some(Self::Track),
            "album" => Some(Self::Album),
            _ => None,
        }
    }
}

/// Reads the summary ffmpeg's `ebur128=peak=true` filter prints.
pub fn parse_ebur128(output: &str) -> Option<Loudness> {
    let summary = &output[output.rfind("Summary:")?..];
    let value = |label: &str| -> Option<f32> {
        summary.lines().find_map(|line| {
            let rest = line.trim().strip_prefix(label)?;
            let number = rest.split_whitespace().next()?;
            if number == "-inf" {
                Some(f32::NEG_INFINITY)
            } else {
                number.parse().ok()
            }
        })
    };
    let integrated = value("I:")?;
    let peak = value("Peak:").unwrap_or(0.0);
    Some(Loudness {
        integrated_lufs: if integrated.is_finite() {
            integrated
        } else {
            SILENCE_LUFS
        },
        true_peak_db: if peak.is_finite() { peak } else { -144.0 },
    })
}

/// Loudness of an album from its tracks' loudness and durations (energy
/// mean, weighted by length), with the loudest peak.
pub fn album_loudness(tracks: &[(Loudness, f64)]) -> Option<Loudness> {
    let total: f64 = tracks.iter().map(|(_, seconds)| seconds.max(1.0)).sum();
    if tracks.is_empty() || total <= 0.0 {
        return None;
    }
    let energy: f64 = tracks
        .iter()
        .map(|(loudness, seconds)| {
            seconds.max(1.0) * 10_f64.powf(f64::from(loudness.integrated_lufs) / 10.0)
        })
        .sum::<f64>()
        / total;
    let peak = tracks
        .iter()
        .map(|(loudness, _)| loudness.true_peak_db)
        .fold(f32::NEG_INFINITY, f32::max);
    Some(Loudness {
        integrated_lufs: (10.0 * energy.log10()) as f32,
        true_peak_db: peak,
    })
}

/// Gain in dB that brings `loudness` to `target_lufs` plus `adjustment_db`,
/// limited so the true peak stays at or below [`PEAK_CEILING_DBTP`].
pub fn gain_db(loudness: Loudness, target_lufs: i8, adjustment_db: i8) -> f32 {
    let desired = f32::from(target_lufs) - loudness.integrated_lufs + f32::from(adjustment_db);
    let headroom = PEAK_CEILING_DBTP - loudness.true_peak_db;
    let gain = desired.min(headroom).clamp(MAX_CUT_DB, MAX_BOOST_DB);
    // Tenths of a dB are plenty and keep stream cache keys stable.
    (gain * 10.0).round() / 10.0
}

/// How the track at `index` of a queue is leveled. `Auto` levels by album
/// while the queue plays an album in order (a neighbour is from the same
/// album) and by track otherwise, as Roon does.
pub fn resolve_mode<F>(
    mode: LevelingMode,
    song_ids: &[String],
    index: usize,
    album_of: F,
) -> Option<LevelBy>
where
    F: Fn(&str) -> Option<String>,
{
    match mode {
        LevelingMode::Off => None,
        LevelingMode::Track => Some(LevelBy::Track),
        LevelingMode::Album => Some(LevelBy::Album),
        LevelingMode::Auto => {
            let album = song_ids.get(index).and_then(|id| album_of(id));
            let same = |neighbour: Option<&String>| {
                album.is_some() && neighbour.and_then(|id| album_of(id)).as_ref() == album.as_ref()
            };
            let previous = index.checked_sub(1).and_then(|before| song_ids.get(before));
            if same(previous) || same(song_ids.get(index + 1)) {
                Some(LevelBy::Album)
            } else {
                Some(LevelBy::Track)
            }
        }
    }
}

/// Identifies one version of a file, so edits are measured again.
fn fingerprint(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Some(format!("{}:{modified}", metadata.len()))
}

/// Runs ffmpeg's EBU R128 meter over the first audio stream.
pub async fn measure_file(path: &Path) -> Option<Loudness> {
    for filter in ["ebur128=peak=true:framelog=quiet", "ebur128=peak=true"] {
        let mut command = tokio::process::Command::new("ffmpeg");
        command
            .kill_on_drop(true)
            .args(["-nostdin", "-hide_banner", "-nostats", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-af", filter, "-f", "null", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let Ok(child) = command.spawn() else {
            return None;
        };
        let output = tokio::time::timeout(MEASURE_TIMEOUT, child.wait_with_output())
            .await
            .ok()?
            .ok()?;
        if output.status.success()
            && let Some(loudness) = parse_ebur128(&String::from_utf8_lossy(&output.stderr))
        {
            return Some(loudness);
        }
    }
    None
}

/// What a client needs to level one track.
#[derive(Clone, Debug, Serialize)]
pub struct TrackLeveling {
    pub track: Option<Loudness>,
    /// Present once every track of the album is measured.
    pub album: Option<Loudness>,
}

/// Measured loudness per song, in memory and in `track_loudness`.
pub struct LoudnessStore {
    pool: Option<DbPool>,
    known: Mutex<HashMap<String, (String, Loudness)>>,
    slots: Arc<Semaphore>,
}

impl LoudnessStore {
    pub fn new(pool: Option<DbPool>) -> Self {
        Self {
            pool,
            known: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(2)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, Loudness)>> {
        self.known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Stored loudness for the current version of the file, if measured.
    pub fn cached(&self, song_id: &str, path: &Path) -> Option<Loudness> {
        let print = fingerprint(path)?;
        if let Some((stored, loudness)) = self.lock().get(song_id)
            && *stored == print
        {
            return Some(*loudness);
        }
        let row = self.pool.as_ref().and_then(|pool| {
            let mut connection = pool.get().ok()?;
            load_row(&mut connection, song_id).ok().flatten()
        })?;
        if row.0 != print {
            return None;
        }
        self.lock().insert(song_id.to_string(), row.clone());
        Some(row.1)
    }

    /// Drops the stored measurement so the next request measures again.
    pub fn forget(&self, song_id: &str) {
        self.lock().remove(song_id);
        if let Some(pool) = &self.pool
            && let Ok(mut connection) = pool.get()
        {
            use diesel::RunQueryDsl;
            let _ = diesel::sql_query("DELETE FROM track_loudness WHERE song_id = ?")
                .bind::<diesel::sql_types::Text, _>(song_id)
                .execute(&mut connection);
        }
    }

    /// Measured loudness, measuring now when needed.
    pub async fn track(&self, song_id: &str, path: &Path) -> Option<Loudness> {
        if let Some(loudness) = self.cached(song_id, path) {
            return Some(loudness);
        }
        let _permit = self.slots.clone().acquire_owned().await.ok()?;
        if let Some(loudness) = self.cached(song_id, path) {
            return Some(loudness);
        }
        let print = fingerprint(path)?;
        let loudness = measure_file(path).await?;
        self.lock()
            .insert(song_id.to_string(), (print.clone(), loudness));
        if let Some(pool) = self.pool.clone() {
            let song_id = song_id.to_string();
            let _ = tokio::task::spawn_blocking(move || {
                let mut connection = pool.get().ok()?;
                save_row(&mut connection, &song_id, &print, loudness).ok()
            })
            .await;
        }
        Some(loudness)
    }

    /// Album loudness when every track of the album is measured.
    pub fn album_cached(&self, cache: &LibraryCache, song_id: &str) -> Option<Loudness> {
        let tracks = album_tracks(cache, song_id)?;
        let measured: Vec<(Loudness, f64)> = tracks
            .iter()
            .map(|(id, path, seconds)| {
                self.cached(id, Path::new(path))
                    .map(|loudness| (loudness, *seconds))
            })
            .collect::<Option<_>>()?;
        album_loudness(&measured)
    }

    /// Track and (when complete) album loudness for `song_id`. Measures the
    /// track now if needed; missing album tracks are measured in the
    /// background so the album value appears shortly after.
    pub async fn leveling_for(
        self: &Arc<Self>,
        cache: &LibraryCache,
        song_id: &str,
    ) -> TrackLeveling {
        let Some(song) = cache.song(song_id) else {
            return TrackLeveling {
                track: None,
                album: None,
            };
        };
        let track = self.track(song_id, Path::new(&song.path)).await;
        let album = self.album_cached(cache, song_id);
        if album.is_none()
            && let Some(tracks) = album_tracks(cache, song_id)
        {
            self.measure_in_background(
                tracks.into_iter().map(|(id, path, _)| (id, path)).collect(),
            );
        }
        TrackLeveling { track, album }
    }

    /// Measures songs that are not measured yet, two at a time.
    pub fn measure_in_background(self: &Arc<Self>, songs: Vec<(String, String)>) {
        let missing: Vec<(String, String)> = songs
            .into_iter()
            .filter(|(id, path)| self.cached(id, Path::new(path)).is_none())
            .collect();
        if missing.is_empty() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let store = self.clone();
        tokio::spawn(async move {
            for (id, path) in missing {
                let _ = store.track(&id, Path::new(&path)).await;
            }
        });
    }

    /// The gain for `song_id` leveled `by` track or album. Album leveling
    /// falls back to the track until the whole album is measured.
    pub async fn gain_for(
        self: &Arc<Self>,
        cache: &LibraryCache,
        song_id: &str,
        by: LevelBy,
        target_lufs: i8,
        adjustment_db: i8,
    ) -> Option<f32> {
        let leveling = self.leveling_for(cache, song_id).await;
        let loudness = match by {
            LevelBy::Album => leveling.album.or(leveling.track),
            LevelBy::Track => leveling.track,
        }?;
        Some(gain_db(loudness, target_lufs, adjustment_db))
    }
}

/// `(song id, path, seconds)` for every track on `song_id`'s album.
fn album_tracks(cache: &LibraryCache, song_id: &str) -> Option<Vec<(String, String, f64)>> {
    let (_, album_id) = cache.song_map.get(song_id)?;
    let album = cache.album(album_id)?;
    let tracks: Vec<(String, String, f64)> = album
        .songs
        .iter()
        .map(|song| (song.id.clone(), song.path.clone(), song.duration))
        .collect();
    (!tracks.is_empty()).then_some(tracks)
}

#[derive(diesel::QueryableByName)]
struct LoudnessRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    fingerprint: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    integrated_lufs: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    true_peak_db: f64,
}

fn load_row(
    connection: &mut diesel::sqlite::SqliteConnection,
    song_id: &str,
) -> Result<Option<(String, Loudness)>, diesel::result::Error> {
    use diesel::RunQueryDsl;
    let rows = diesel::sql_query(
        "SELECT fingerprint, integrated_lufs, true_peak_db FROM track_loudness WHERE song_id = ?",
    )
    .bind::<diesel::sql_types::Text, _>(song_id)
    .load::<LoudnessRow>(connection)?;
    Ok(rows.into_iter().next().map(|row| {
        (
            row.fingerprint,
            Loudness {
                integrated_lufs: row.integrated_lufs as f32,
                true_peak_db: row.true_peak_db as f32,
            },
        )
    }))
}

fn save_row(
    connection: &mut diesel::sqlite::SqliteConnection,
    song_id: &str,
    fingerprint: &str,
    loudness: Loudness,
) -> Result<(), diesel::result::Error> {
    use diesel::RunQueryDsl;
    use diesel::sql_types::{Double, Text};
    diesel::sql_query(
        "INSERT INTO track_loudness (song_id, fingerprint, integrated_lufs, true_peak_db, measured_at)
         VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(song_id) DO UPDATE SET fingerprint = excluded.fingerprint,
             integrated_lufs = excluded.integrated_lufs, true_peak_db = excluded.true_peak_db,
             measured_at = CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(song_id)
    .bind::<Text, _>(fingerprint)
    .bind::<Double, _>(f64::from(loudness.integrated_lufs))
    .bind::<Double, _>(f64::from(loudness.true_peak_db))
    .execute(connection)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMMARY: &str = "[Parsed_ebur128_0 @ 0x55] Summary:\n\n  Integrated loudness:\n    I:         -27.8 LUFS\n    Threshold: -37.8 LUFS\n\n  Loudness range:\n    LRA:         0.0 LU\n\n  True peak:\n    Peak:      -24.1 dBFS\n";

    #[test]
    fn reads_the_ebur128_summary() {
        let loudness = parse_ebur128(SUMMARY).expect("summary");
        assert!((loudness.integrated_lufs + 27.8).abs() < 0.01);
        assert!((loudness.true_peak_db + 24.1).abs() < 0.01);
        let silent =
            parse_ebur128("Summary:\n    I:         -inf LUFS\n    Peak:       -inf dBFS\n")
                .expect("silence");
        assert_eq!(silent.integrated_lufs, SILENCE_LUFS);
        assert!(parse_ebur128("no summary here").is_none());
    }

    #[test]
    fn gain_reaches_the_target_without_clipping() {
        let loud = Loudness {
            integrated_lufs: -8.0,
            true_peak_db: 0.5,
        };
        assert_eq!(gain_db(loud, -18, 0), -10.0);
        assert_eq!(gain_db(loud, -18, 3), -7.0);
        // A quiet track is raised only until its peak reaches -1 dBTP.
        let quiet = Loudness {
            integrated_lufs: -28.0,
            true_peak_db: -6.0,
        };
        assert_eq!(gain_db(quiet, -18, 0), 5.0);
        // Never more than the boost limit.
        let silent = Loudness {
            integrated_lufs: -70.0,
            true_peak_db: -60.0,
        };
        assert_eq!(gain_db(silent, -18, 0), MAX_BOOST_DB);
    }

    #[test]
    fn album_loudness_is_an_energy_mean_with_the_loudest_peak() {
        let tracks = [
            (
                Loudness {
                    integrated_lufs: -10.0,
                    true_peak_db: -1.0,
                },
                100.0,
            ),
            (
                Loudness {
                    integrated_lufs: -20.0,
                    true_peak_db: -3.0,
                },
                100.0,
            ),
        ];
        let album = album_loudness(&tracks).expect("album");
        assert!((album.integrated_lufs + 12.6).abs() < 0.1, "{album:?}");
        assert_eq!(album.true_peak_db, -1.0);
        assert!(album_loudness(&[]).is_none());
    }

    #[test]
    fn auto_levels_by_album_only_while_an_album_plays_in_order() {
        let queue: Vec<String> = ["a1", "a2", "b1", "c1"].map(String::from).to_vec();
        let album_of = |id: &str| Some(id[..1].to_string());
        let auto = |index| resolve_mode(LevelingMode::Auto, &queue, index, album_of);
        assert_eq!(auto(0), Some(LevelBy::Album));
        assert_eq!(auto(1), Some(LevelBy::Album));
        assert_eq!(auto(2), Some(LevelBy::Track));
        assert_eq!(auto(3), Some(LevelBy::Track));
        assert_eq!(resolve_mode(LevelingMode::Off, &queue, 0, album_of), None);
        assert_eq!(
            resolve_mode(LevelingMode::Album, &queue, 3, album_of),
            Some(LevelBy::Album)
        );
    }
}
