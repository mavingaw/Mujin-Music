//! The interface the zone service drives server-played outputs through.

use futures::future::BoxFuture;
use serde::Serialize;

use super::FormatLimits;
use crate::zones::{DeviceVolumeRange, VolumeMode, ZoneDsp, ZoneKind, ZoneStreamInfo};

/// Transport state of a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
    Transitioning,
}

/// Technical format of a source file, from the library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceFormat {
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
}

impl SourceFormat {
    pub fn lossless(&self) -> bool {
        matches!(
            self.codec.as_deref(),
            Some("flac" | "alac" | "pcm" | "wav" | "aiff" | "ape" | "wavpack" | "wv")
        )
    }
}

/// One track handed to a renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackRequest {
    /// Unique per request, so the zone service can tell which queued
    /// track the renderer is on (even when the same song repeats).
    pub serial: u64,
    pub song_id: String,
    /// File on this machine (device outputs decode it directly).
    pub path: String,
    pub duration_ms: i64,
    pub source: SourceFormat,
    /// Signed media URL (network renderers fetch it).
    pub uri: String,
    /// DIDL-Lite metadata for `uri`.
    pub metadata: String,
    pub mime_type: String,
    /// Display metadata (Chromecast shows it on screen).
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Signed cover URL, for network renderers.
    pub art_uri: Option<String>,
    /// Volume leveling gain for this track, in dB.
    pub gain_db: Option<f32>,
    pub dsp: ZoneDsp,
}

/// Static facts about a renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererInfo {
    /// Zone id: `upnp:<udn>` or `device:<id>`.
    pub id: String,
    pub kind: ZoneKind,
    pub name: String,
    /// e.g. `Denon AVR-X2800H` or `USB Audio · hw:2,0`.
    pub device: String,
    pub limits: FormatLimits,
    /// The device's own volume scale.
    pub volume_range: Option<DeviceVolumeRange>,
    /// Volume modes the renderer offers.
    pub volume_modes: Vec<VolumeMode>,
    /// Plays the next track without a gap.
    pub gapless: bool,
    /// Can be opened exclusively (local devices).
    pub exclusive_capable: bool,
    /// The renderer fetches signed URLs (network) rather than files.
    pub network: bool,
    pub notes: Vec<String>,
}

/// What a renderer reports.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererState {
    pub state: PlayState,
    /// The Parson song the renderer is on, when it plays one of ours.
    pub song_id: Option<String>,
    /// [`TrackRequest::serial`] of that track.
    pub serial: Option<u64>,
    /// Something another app started.
    pub foreign: Option<String>,
    pub position_ms: i64,
    pub duration_ms: i64,
    /// Unix milliseconds when `position_ms` was read.
    pub polled_at_ms: i64,
    /// 0..=100 on Parson's scale.
    pub volume: u8,
    pub muted: bool,
    /// The device's current value on its own scale.
    pub device_volume: Option<f32>,
    /// Serial of a track that played to its end with nothing queued after.
    pub finished_serial: Option<u64>,
    pub reachable: bool,
    /// Signal path of the playing track, when the renderer knows it.
    pub stream: Option<ZoneStreamInfo>,
}

impl Default for RendererState {
    fn default() -> Self {
        Self {
            state: PlayState::Stopped,
            song_id: None,
            serial: None,
            foreign: None,
            position_ms: 0,
            duration_ms: 0,
            polled_at_ms: 0,
            volume: 50,
            muted: false,
            device_volume: None,
            finished_serial: None,
            reachable: true,
            stream: None,
        }
    }
}

/// Per-zone settings a renderer applies itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RendererSettings {
    pub exclusive: bool,
    pub volume_mode: VolumeMode,
    pub limits: FormatLimits,
}

/// A server-driven output. Every call is quick: devices queue the work on
/// their playback thread, network renderers make one or two SOAP calls.
pub trait Renderer: Send + Sync {
    fn info(&self) -> RendererInfo;
    fn state(&self) -> RendererState;
    /// Applies settings from the zone (exclusive mode, volume mode).
    fn configure(&self, settings: RendererSettings);
    /// This server's address as the renderer reaches it, for media URLs.
    fn media_origin(&self, _port: u16) -> Option<String> {
        None
    }
    /// Starts `track` at `position_ms` (paused unless `play`); `next`
    /// follows gaplessly when the renderer can.
    fn load(
        &self,
        track: TrackRequest,
        next: Option<TrackRequest>,
        position_ms: i64,
        play: bool,
    ) -> BoxFuture<'_, Result<(), String>>;
    /// Replaces the track that follows the current one.
    fn set_next(&self, next: Option<TrackRequest>) -> BoxFuture<'_, Result<(), String>>;
    fn play(&self) -> BoxFuture<'_, Result<(), String>>;
    fn pause(&self) -> BoxFuture<'_, Result<(), String>>;
    fn stop(&self) -> BoxFuture<'_, Result<(), String>>;
    fn seek(&self, position_ms: i64) -> BoxFuture<'_, Result<(), String>>;
    /// `level` is on Parson's 0..=100 scale.
    fn set_volume(&self, level: u8) -> BoxFuture<'_, Result<(), String>>;
    fn set_mute(&self, muted: bool) -> BoxFuture<'_, Result<(), String>>;
    /// Refreshes [`Renderer::state`].
    fn poll(&self) -> BoxFuture<'_, ()>;
    /// Zone ids of the members, for a group that plays as one renderer.
    fn group_members(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Most of a dB scale the 0..=100 slider spans: below -80 dB is silence in
/// practice, and a linear map over a DAC's full -127 dB would put half the
/// slider in inaudible territory.
const MAX_SLIDER_SPAN_DB: f32 = 80.0;

fn usable_min(range: &DeviceVolumeRange) -> f32 {
    if range.unit == "dB" {
        range.min.max(range.max - MAX_SLIDER_SPAN_DB)
    } else {
        range.min
    }
}

/// Maps Parson's 0..=100 to a device scale, snapped to its step.
pub fn to_device_scale(level: u8, range: &DeviceVolumeRange) -> f32 {
    let fraction = f32::from(level.min(100)) / 100.0;
    let low = usable_min(range);
    if level == 0 {
        return range.min;
    }
    let raw = low + (range.max - low) * fraction;
    if range.step > 0.0 {
        let steps = ((raw - range.min) / range.step).round();
        (range.min + steps * range.step).clamp(range.min, range.max)
    } else {
        raw
    }
}

/// Maps a device value back to Parson's 0..=100.
pub fn from_device_scale(value: f32, range: &DeviceVolumeRange) -> u8 {
    let low = usable_min(range);
    if range.max <= low {
        return 100;
    }
    (((value - low) / (range.max - low)) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::{from_device_scale, to_device_scale};
    use crate::zones::DeviceVolumeRange;

    #[test]
    fn volume_maps_onto_device_scales_and_back() {
        let steps = DeviceVolumeRange {
            min: 0.0,
            max: 30.0,
            step: 1.0,
            unit: "steps",
            hardware: true,
            current: None,
        };
        assert_eq!(to_device_scale(50, &steps), 15.0);
        assert_eq!(to_device_scale(51, &steps), 15.0);
        assert_eq!(to_device_scale(100, &steps), 30.0);
        assert_eq!(from_device_scale(15.0, &steps), 50);
        let decibels = DeviceVolumeRange {
            min: -127.5,
            max: 0.0,
            step: 0.5,
            unit: "dB",
            hardware: true,
            current: None,
        };
        assert_eq!(to_device_scale(0, &decibels), -127.5);
        assert_eq!(to_device_scale(80, &decibels), -16.0);
        assert_eq!(to_device_scale(100, &decibels), 0.0);
        assert_eq!(from_device_scale(-16.0, &decibels), 80);
        assert_eq!(from_device_scale(-127.5, &decibels), 0);
    }
}
