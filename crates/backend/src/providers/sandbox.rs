//! Sandbox mode (`PARSON_STREAMING_SANDBOX=1`).
//!
//! Runs the real TIDAL and Qobuz clients against the recorded fixtures used
//! by the tests, so the whole sign-in, browse, library and playback flow can
//! be tried without an account. Nothing leaves the server: artwork is drawn
//! locally and "streams" are short synthesized tones. The UI labels every
//! sandbox screen as such.

use super::transport::FixtureTransport;
use super::{SearchResults, StreamQuality};
use crate::library::search::normalize;

pub const TIDAL_FIXTURES: &str = include_str!("fixtures/tidal.json");
pub const QOBUZ_FIXTURES: &str = include_str!("fixtures/qobuz.json");

/// Length of a sandbox preview.
pub const PREVIEW_SECONDS: u32 = 30;
const SAMPLE_RATE: u32 = 44_100;

pub fn enabled_from_env() -> bool {
    std::env::var("PARSON_STREAMING_SANDBOX")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub fn transport() -> FixtureTransport {
    FixtureTransport::from_json(&[TIDAL_FIXTURES, QOBUZ_FIXTURES])
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

/// A gentle pentatonic arpeggio, seeded by the track id, as a 16-bit mono
/// WAV. Higher chosen qualities get no extra fidelity: this is a stand-in for
/// a real stream, and the UI says so.
pub fn preview_wav(seed: u64, seconds: u32, _quality: StreamQuality) -> Vec<u8> {
    const SCALE: [f32; 10] = [
        220.0, 246.94, 277.18, 329.63, 369.99, 440.0, 493.88, 554.37, 659.25, 739.99,
    ];
    let samples = (SAMPLE_RATE * seconds) as usize;
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    let root = SCALE[next() % 5];
    let note_length = SAMPLE_RATE as usize * (3 + next() % 3) / 8;
    let notes = (0..samples / note_length + 1)
        .map(|_| SCALE[next() % SCALE.len()])
        .collect::<Vec<_>>();

    let mut pcm = Vec::with_capacity(samples * 2);
    for index in 0..samples {
        let time = index as f32 / SAMPLE_RATE as f32;
        let note = notes[index / note_length];
        let since = (index % note_length) as f32 / SAMPLE_RATE as f32;
        let pluck = (-since * 3.2).exp()
            * ((std::f32::consts::TAU * note * time).sin()
                + 0.3 * (std::f32::consts::TAU * note * 2.0 * time).sin());
        let pad = 0.35
            * ((std::f32::consts::TAU * root * 0.5 * time).sin()
                + 0.6 * (std::f32::consts::TAU * root * 0.75 * time).sin());
        let fade_in = (time / 1.5).min(1.0);
        let fade_out = ((seconds as f32 - time) / 2.0).clamp(0.0, 1.0);
        let value = (0.28 * pluck + 0.12 * pad) * fade_in * fade_out;
        pcm.extend_from_slice(&((value.clamp(-1.0, 1.0) * 30_000.0) as i16).to_le_bytes());
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
