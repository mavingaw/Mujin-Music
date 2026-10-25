//! Sandbox mode (`PARSON_STREAMING_SANDBOX=1`).
//!
//! Runs the real TIDAL and Qobuz clients against the recorded fixtures used
//! by the tests, so the whole sign-in, browse, library and playback flow can
//! be tried without an account. Nothing leaves the server: artwork is drawn
//! locally and "streams" are short synthesized tones, rendered by
//! [`super::sandbox_media`] in the format each recorded reply announces
//! (24/192 FLAC, TIDAL Max DASH, 16/44.1 FLAC, AAC, MP3). The UI labels
//! every sandbox screen as such.

use super::transport::FixtureTransport;
use super::{SearchResults, StreamQuality};
use crate::library::search::normalize;

pub const TIDAL_FIXTURES: &str = include_str!("fixtures/tidal.json");
pub const QOBUZ_FIXTURES: &str = include_str!("fixtures/qobuz.json");
/// Genre and editorial lists. Loaded first: their routes are more specific
/// than the generic recorded `album/getFeatured` reply.
pub const TIDAL_BROWSE_FIXTURES: &str = include_str!("fixtures/tidal_browse.json");
pub const QOBUZ_BROWSE_FIXTURES: &str = include_str!("fixtures/qobuz_browse.json");
/// Reviews, credits, biographies, similar artists and radios.
pub const TIDAL_NOTES_FIXTURES: &str = include_str!("fixtures/tidal_notes.json");
pub const QOBUZ_NOTES_FIXTURES: &str = include_str!("fixtures/qobuz_notes.json");

/// Length of a sandbox preview.
pub const PREVIEW_SECONDS: u32 = 30;
const SAMPLE_RATE: u32 = 44_100;

pub fn enabled_from_env() -> bool {
    std::env::var("PARSON_STREAMING_SANDBOX")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub fn transport() -> FixtureTransport {
    FixtureTransport::from_json(&[
        TIDAL_BROWSE_FIXTURES,
        QOBUZ_BROWSE_FIXTURES,
        TIDAL_NOTES_FIXTURES,
        QOBUZ_NOTES_FIXTURES,
        TIDAL_FIXTURES,
        QOBUZ_FIXTURES,
    ])
    .expect("embedded streaming fixtures are valid JSON")
}

/// Recorded search responses are not query-specific, so sandbox search keeps
/// only results whose title, artist or album contains every query word.
pub fn filter_search(results: SearchResults, query: &str) -> SearchResults {
    let words = normalize(query)
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let matches = |fields: &[&str]| {
        let haystack = normalize(&fields.join(" "));
        words.iter().all(|word| haystack.contains(word.as_str()))
    };
    SearchResults {
        albums: results
            .albums
            .into_iter()
            .filter(|album| matches(&[&album.title, &album.artist.name]))
            .collect(),
        tracks: results
            .tracks
            .into_iter()
            .filter(|track| {
                matches(&[
                    &track.title,
                    &track.artist.name,
                    track
                        .album
                        .as_ref()
                        .map_or("", |album| album.title.as_str()),
                ])
            })
            .collect(),
        artists: results
            .artists
            .into_iter()
            .filter(|artist| matches(&[&artist.name]))
            .collect(),
    }
}

/// Seeds a sandbox track's tone from its id, so a track always sounds the
/// same.
pub fn seed(raw_id: &str) -> u64 {
    raw_id.bytes().fold(1469598103934665603_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(1099511628211)
    })
}

/// A gentle pentatonic arpeggio over a soft pad, seeded by the track id.
/// The sandbox renders it at whatever rate a stream needs, so a 24/192
/// sandbox stream really carries 192,000 samples a second.
pub struct Tone {
    rate: u32,
    seconds: u32,
    root: f32,
    note_length: usize,
    notes: Vec<f32>,
}

impl Tone {
    pub fn new(seed: u64, seconds: u32, rate: u32) -> Self {
        const SCALE: [f32; 10] = [
            220.0, 246.94, 277.18, 329.63, 369.99, 440.0, 493.88, 554.37, 659.25, 739.99,
        ];
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        let root = SCALE[next() % 5];
        let note_length = (rate as usize * (3 + next() % 3) / 8).max(1);
        let samples = rate as usize * seconds as usize;
        let notes = (0..samples / note_length + 1)
            .map(|_| SCALE[next() % SCALE.len()])
            .collect::<Vec<_>>();
        Self {
            rate,
            seconds,
            root,
            note_length,
            notes,
        }
    }

    pub fn frames(&self) -> usize {
        self.rate as usize * self.seconds as usize
    }

    /// One stereo frame in -1.0..=1.0. Notes alternate gently left and
    /// right over a centred pad.
    pub fn frame(&self, index: usize) -> (f32, f32) {
        let rate = self.rate as f32;
        let time = index as f32 / rate;
        let note_index = index / self.note_length;
        let note = self.notes[note_index.min(self.notes.len() - 1)];
        let since = (index % self.note_length) as f32 / rate;
        let pluck = (-since * 3.2).exp()
            * ((std::f32::consts::TAU * note * time).sin()
                + 0.3 * (std::f32::consts::TAU * note * 2.0 * time).sin());
        let pad = 0.35
            * ((std::f32::consts::TAU * self.root * 0.5 * time).sin()
                + 0.6 * (std::f32::consts::TAU * self.root * 0.75 * time).sin());
        let fade_in = (time / 1.5).min(1.0);
        let fade_out = ((self.seconds as f32 - time) / 2.0).clamp(0.0, 1.0);
        let pan = if note_index.is_multiple_of(2) {
            0.25
        } else {
            -0.25
        };
        let gain = fade_in * fade_out;
        let left = (0.28 * pluck * (1.0 - pan) + 0.12 * pad) * gain;
        let right = (0.28 * pluck * (1.0 + pan) + 0.12 * pad) * gain;
        (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0))
    }
}

/// The tone as a 16-bit mono WAV at 44.1 kHz: the fallback "stream" when
/// the server cannot render sandbox audio (no ffmpeg). Higher chosen
/// qualities get no extra fidelity, and the UI says so.
pub fn preview_wav(seed: u64, seconds: u32, _quality: StreamQuality) -> Vec<u8> {
    let tone = Tone::new(seed, seconds, SAMPLE_RATE);
    let mut pcm = Vec::with_capacity(tone.frames() * 2);
    for index in 0..tone.frames() {
        let (left, right) = tone.frame(index);
        let value = (left + right) / 2.0;
        pcm.extend_from_slice(&((value * 30_000.0) as i16).to_le_bytes());
    }

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

#[cfg(test)]
mod tests {
    use super::{filter_search, preview_wav, transport};
    use crate::providers::{ArtistSummary, ProviderId, SearchResults, StreamQuality};

    #[test]
    fn embedded_fixtures_load() {
        assert!(transport().requests().is_empty());
    }

    #[test]
    fn preview_audio_is_a_valid_bounded_wav() {
        let wav = preview_wav(7, 2, StreamQuality::Lossless);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 2 * 44_100 * 2);
        assert_ne!(wav, preview_wav(8, 2, StreamQuality::Lossless));
    }

    #[test]
    fn sandbox_search_keeps_only_matching_results() {
        let results = SearchResults {
            albums: Vec::new(),
            tracks: Vec::new(),
            artists: ["Mara Vell", "Sora Ueda"]
                .into_iter()
                .map(|name| ArtistSummary {
                    id: "tidal:1".into(),
                    provider: ProviderId::Tidal,
                    name: name.into(),
                    picture_url: None,
                })
                .collect(),
        };
        let filtered = filter_search(results, "mara");
        assert_eq!(filtered.artists.len(), 1);
        assert_eq!(filtered.artists[0].name, "Mara Vell");
    }
}
