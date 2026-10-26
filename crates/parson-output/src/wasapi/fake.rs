//! A protocol-level fake of WASAPI for tests and demos on machines that
//! are not Windows. It enforces the rules real drivers do, as Microsoft
//! documents them for `IAudioClient`:
//!
//! - exclusive mode takes only the formats the endpoint lists, exactly;
//!   anything else is `AUDCLNT_E_UNSUPPORTED_FORMAT`;
//! - an exclusive buffer whose byte size is not aligned (128 bytes, as on
//!   HD Audio) fails `Initialize` with `AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED`;
//!   `GetBufferSize` then reports the aligned size, and the client cannot be
//!   initialised again (`AUDCLNT_E_ALREADY_INITIALIZED`): a fresh one must
//!   be activated;
//! - event-driven exclusive streams take exactly one whole buffer per
//!   `GetBuffer` (`AUDCLNT_E_INVALID_SIZE` otherwise) and only when one of
//!   the two buffers is free (`AUDCLNT_E_BUFFER_TOO_LARGE` otherwise);
//! - one exclusive client per endpoint (`AUDCLNT_E_DEVICE_IN_USE`), and
//!   none at all when exclusive mode is turned off for the device;
//! - `Reset` only on a stopped stream (`AUDCLNT_E_NOT_STOPPED`);
//! - the device plays in real time (optionally sped up) from `Start`, and
//!   `IAudioClock` reports what it has played.
//!
//! It keeps what each endpoint played so tests can prove bit-perfection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    AUDCLNT_E_ALREADY_INITIALIZED, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_BUFFER_TOO_LARGE,
    AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED, AUDCLNT_E_INVALID_SIZE,
    AUDCLNT_E_NOT_INITIALIZED, AUDCLNT_E_NOT_STOPPED, AUDCLNT_E_UNSUPPORTED_FORMAT, E_INVALIDARG,
    E_NOTFOUND, Endpoint, EndpointVolume, HResult, S_FALSE, S_OK, ShareMode, WasapiApi,
    WasapiClient, WaveFormat,
};

/// One fake render endpoint.
#[derive(Clone, Debug)]
pub struct FakeEndpoint {
    pub endpoint: Endpoint,
    /// What exclusive mode takes, exactly.
    pub exclusive_formats: Vec<WaveFormat>,
    pub volume: Option<EndpointVolume>,
    /// "Allow applications to take exclusive control of this device".
    pub exclusive_allowed: bool,
    /// Another app holds the device in exclusive mode.
    pub held_by_other_app: bool,
    /// Buffer byte sizes must be a multiple of this in exclusive mode.
    pub alignment_bytes: usize,
    /// `GetDevicePeriod` (default, minimum), 100 ns units.
    pub period_hns: (i64, i64),
}

impl FakeEndpoint {
    /// A hi-res USB DAC: 44.1 to 384 kHz at 16 and 24 bits, and 24 bits in
    /// a 32-bit container (no 32-bit valid samples), with a hardware volume
    /// control of -127.5 to 0 dB in 0.5 dB steps.
    pub fn usb_dac(guid: &str, name: &str) -> Self {
        let mut exclusive_formats = Vec::new();
        for rate in [
            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
        ] {
            for (container_bits, valid_bits) in [(16, 16), (24, 24), (32, 24)] {
                exclusive_formats.push(WaveFormat {
                    sample_rate: rate,
                    channels: 2,
                    container_bits,
                    valid_bits,
                });
            }
        }
        Self {
            endpoint: Endpoint {
                id: format!("{{0.0.0.00000000}}.{{{guid}}}"),
                name: format!("Speakers ({name})"),
                adapter: name.to_string(),
                mix: WaveFormat {
                    sample_rate: 48_000,
                    channels: 2,
                    container_bits: 32,
                    valid_bits: 32,
                },
            },
            exclusive_formats,
            volume: Some(EndpointVolume {
                min_db: -127.5,
                max_db: 0.0,
                step_db: 0.5,
                hardware: true,
            }),
            exclusive_allowed: true,
            held_by_other_app: false,
            alignment_bytes: 128,
            period_hns: (100_000, 30_000),
        }
    }

    /// Laptop speakers: 48 kHz/16 and 24-bit only, volume done in software.
    pub fn laptop_speakers(guid: &str) -> Self {
        let mut speakers = Self::usb_dac(guid, "Realtek(R) Audio");
        speakers.exclusive_formats = [(16, 16), (24, 24)]
            .into_iter()
            .map(|(container_bits, valid_bits)| WaveFormat {
                sample_rate: 48_000,
                channels: 2,
                container_bits,
                valid_bits,
            })
            .collect();
        speakers.volume = Some(EndpointVolume {
            min_db: -65.25,
            max_db: 0.0,
            step_db: 0.03125,
            hardware: false,
        });
        speakers
    }
}

/// What one endpoint went through.
#[derive(Clone, Debug, Default)]
pub struct FakeCapture {
    /// Every successful `Initialize`: format, mode, buffer frames.
    pub opened: Vec<(WaveFormat, ShareMode, u32)>,
    /// `IMMDevice::Activate` calls.
    pub activations: u32,
    /// `Initialize` calls that failed with `BUFFER_SIZE_NOT_ALIGNED`.
    pub alignment_retries: u32,
    /// What the device played (or holds to play), in order.
    pub bytes: Vec<u8>,
    pub volume_db: Option<f32>,
    pub muted: bool,
    /// Times the device ran out of audio while running.
    pub glitches: u32,
}

#[derive(Default)]
struct State {
    endpoints: Vec<FakeEndpoint>,
    captures: HashMap<String, FakeCapture>,
    /// Exclusive client per endpoint id.
    holders: HashMap<String, u64>,
}

pub struct FakeWasapi {
    state: Arc<Mutex<State>>,
    speed: f64,
}

static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl FakeWasapi {
    /// `speed` 1.0 plays in real time.
    pub fn new(endpoints: Vec<FakeEndpoint>, speed: f64) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                endpoints,
                ..State::default()
            })),
            speed: speed.max(0.01),
        }
    }

    pub fn capture(&self, endpoint_id: &str) -> FakeCapture {
        lock(&self.state)
            .captures
            .get(endpoint_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Changes an endpoint (another app takes it, exclusive is turned off).
    pub fn update(&self, endpoint_id: &str, change: impl FnOnce(&mut FakeEndpoint)) {
        let mut state = lock(&self.state);
        if let Some(endpoint) = state
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.endpoint.id == endpoint_id)
        {
            change(endpoint);
        }
    }

    pub fn unplug(&self, endpoint_id: &str) {
        lock(&self.state)
            .endpoints
            .retain(|endpoint| endpoint.endpoint.id != endpoint_id);
    }

    fn endpoint(&self, id: &str) -> Result<FakeEndpoint, HResult> {
        lock(&self.state)
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint.id == id)
            .cloned()
            .ok_or(E_NOTFOUND)
    }
}

fn exclusive_check(state: &State, endpoint: &FakeEndpoint, client: Option<u64>) -> HResult {
    if !endpoint.exclusive_allowed {
        return AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED;
    }
    let held = state
        .holders
        .get(&endpoint.endpoint.id)
        .is_some_and(|holder| Some(*holder) != client);
    if endpoint.held_by_other_app || held {
        return AUDCLNT_E_DEVICE_IN_USE;
    }
    S_OK
}

impl WasapiApi for FakeWasapi {
    type Client = FakeClient;

    fn endpoints(&self) -> Result<Vec<Endpoint>, HResult> {
        Ok(lock(&self.state)
            .endpoints
            .iter()
            .map(|endpoint| endpoint.endpoint.clone())
            .collect())
    }

    fn is_format_supported(&self, id: &str, mode: ShareMode, format: &WaveFormat) -> HResult {
        let Ok(endpoint) = self.endpoint(id) else {
            return E_NOTFOUND;
        };
        match mode {
            ShareMode::Exclusive => {
                let state = lock(&self.state);
                let check = exclusive_check(&state, &endpoint, None);
                if check != S_OK {
                    return check;
                }
                if endpoint.exclusive_formats.contains(format) {
                    S_OK
                } else {
                    AUDCLNT_E_UNSUPPORTED_FORMAT
                }
            }
            // Shared mode: the mix format, or the closest match.
            ShareMode::Shared => {
                if *format == endpoint.endpoint.mix {
                    S_OK
                } else {
                    S_FALSE
                }
            }
        }
    }

    fn endpoint_volume(&self, id: &str) -> Result<EndpointVolume, HResult> {
        self.endpoint(id)?.volume.ok_or(E_NOTFOUND)
    }

    fn set_volume_db(&self, id: &str, db: f32) -> Result<(), HResult> {
        let volume = self.endpoint_volume(id)?;
        if !(volume.min_db..=volume.max_db).contains(&db) {
            return Err(E_INVALIDARG);
        }
        lock(&self.state)
            .captures
            .entry(id.to_string())
            .or_default()
            .volume_db = Some(db);
        Ok(())
    }

    fn set_mute(&self, id: &str, muted: bool) -> Result<(), HResult> {
        self.endpoint(id)?;
        lock(&self.state)
            .captures
            .entry(id.to_string())
            .or_default()
            .muted = muted;
        Ok(())
    }

    fn activate(&self, id: &str) -> Result<FakeClient, HResult> {
        let endpoint = self.endpoint(id)?;
        lock(&self.state)
            .captures
            .entry(id.to_string())
            .or_default()
            .activations += 1;
        Ok(FakeClient {
            serial: NEXT_CLIENT.fetch_add(1, Ordering::Relaxed),
            endpoint,
            state: self.state.clone(),
            speed: self.speed,
            phase: Phase::New,
            stream: None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    New,
    /// `Initialize` failed with `BUFFER_SIZE_NOT_ALIGNED`.
    NotAligned(u32),
    Ready,
}

struct Stream {
    format: WaveFormat,
    mode: ShareMode,
    event_driven: bool,
    buffer_frames: u32,
    /// Frames handed over since the last reset.
    rendered: u64,
    /// Frames played before `started`.
    played_before: f64,
    started: Option<Instant>,
}

impl Stream {
    fn played(&self, speed: f64) -> u64 {
        let running = self
            .started
            .map(|started| {
                started.elapsed().as_secs_f64() * f64::from(self.format.sample_rate) * speed
            })
            .unwrap_or(0.0);
        ((self.played_before + running) as u64).min(self.rendered)
    }
}

pub struct FakeClient {
    serial: u64,
    endpoint: FakeEndpoint,
    state: Arc<Mutex<State>>,
    speed: f64,
    phase: Phase,
    stream: Option<Stream>,
}

impl FakeClient {
    fn stream(&self) -> Result<&Stream, HResult> {
        self.stream.as_ref().ok_or(AUDCLNT_E_NOT_INITIALIZED)
    }

    fn queued(&self) -> u64 {
        self.stream
            .as_ref()
            .map(|stream| stream.rendered - stream.played(self.speed))
            .unwrap_or(0)
    }
}

impl WasapiClient for FakeClient {
    fn device_period(&self) -> Result<(i64, i64), HResult> {
        Ok(self.endpoint.period_hns)
    }

    fn initialize(
        &mut self,
        mode: ShareMode,
        event_driven: bool,
        buffer_hns: i64,
        period_hns: i64,
        format: &WaveFormat,
    ) -> Result<(), HResult> {
        if self.phase != Phase::New {
            return Err(AUDCLNT_E_ALREADY_INITIALIZED);
        }
        if format.channels == 0 || format.sample_rate == 0 {
            return Err(E_INVALIDARG);
        }
        let mut state = lock(&self.state);
        let id = self.endpoint.endpoint.id.clone();
        let buffer_frames = match mode {
            ShareMode::Exclusive => {
                let check = exclusive_check(&state, &self.endpoint, Some(self.serial));
                if check != S_OK {
                    return Err(check);
                }
                if !self.endpoint.exclusive_formats.contains(format) {
                    return Err(AUDCLNT_E_UNSUPPORTED_FORMAT);
                }
                if event_driven && buffer_hns != period_hns {
                    return Err(E_INVALIDARG);
                }
                if period_hns < self.endpoint.period_hns.1 {
                    return Err(E_INVALIDARG);
                }
                let frames =
                    (period_hns as f64 * f64::from(format.sample_rate) / 1e7).round() as u32;
                let block = format.block_align();
                if !(frames as usize * block).is_multiple_of(self.endpoint.alignment_bytes) {
                    let mut aligned = frames;
                    while !(aligned as usize * block).is_multiple_of(self.endpoint.alignment_bytes)
                    {
                        aligned += 1;
                    }
                    self.phase = Phase::NotAligned(aligned);
                    state.captures.entry(id).or_default().alignment_retries += 1;
                    return Err(AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED);
                }
                state.holders.insert(id.clone(), self.serial);
                frames
            }
            // AUTOCONVERTPCM: Windows converts to the mix format.
            ShareMode::Shared => {
                (buffer_hns as f64 * f64::from(format.sample_rate) / 1e7).round() as u32
            }
        };
        state
            .captures
            .entry(id)
            .or_default()
            .opened
            .push((*format, mode, buffer_frames));
        self.phase = Phase::Ready;
        self.stream = Some(Stream {
            format: *format,
            mode,
            event_driven,
            buffer_frames,
            rendered: 0,
            played_before: 0.0,
            started: None,
        });
        Ok(())
    }

    fn buffer_frames(&self) -> Result<u32, HResult> {
        match self.phase {
            Phase::NotAligned(frames) => Ok(frames),
            _ => Ok(self.stream()?.buffer_frames),
        }
    }

    fn padding_frames(&self) -> Result<u32, HResult> {
        self.stream()?;
        Ok(self.queued() as u32)
    }

    fn wait(&self, timeout_ms: u32) -> bool {
        let Some(stream) = &self.stream else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        // Signalled when one of the two buffers has played.
        loop {
            if self.queued() <= u64::from(stream.buffer_frames) {
                return true;
            }
            if stream.started.is_none() || Instant::now() >= deadline {
                if stream.started.is_none() {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
                return false;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    }

    fn render(&mut self, frames: u32, data: &[u8]) -> Result<(), HResult> {
        let queued = self.queued();
        let speed = self.speed;
        let stream = self.stream.as_mut().ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        // The same contract the real client checks before GetBuffer.
        if !super::render_fits(frames, stream.format.block_align(), data.len()) {
            return Err(E_INVALIDARG);
        }
        match (stream.mode, stream.event_driven) {
            (ShareMode::Exclusive, true) => {
                if frames != stream.buffer_frames {
                    return Err(AUDCLNT_E_INVALID_SIZE);
                }
                if queued > u64::from(stream.buffer_frames) {
                    return Err(AUDCLNT_E_BUFFER_TOO_LARGE);
                }
            }
            _ => {
                if queued + u64::from(frames) > u64::from(stream.buffer_frames) {
                    return Err(AUDCLNT_E_BUFFER_TOO_LARGE);
                }
            }
        }
        let starved = stream.started.is_some() && stream.played(speed) >= stream.rendered;
        stream.rendered += u64::from(frames);
        let mut state = lock(&self.state);
        let capture = state
            .captures
            .entry(self.endpoint.endpoint.id.clone())
            .or_default();
        if starved {
            capture.glitches += 1;
        }
        capture.bytes.extend_from_slice(data);
        Ok(())
    }

    fn start(&mut self) -> Result<(), HResult> {
        let stream = self.stream.as_mut().ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        if stream.started.is_none() {
            stream.started = Some(Instant::now());
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), HResult> {
        let speed = self.speed;
        let stream = self.stream.as_mut().ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        // Stopping a stopped stream is S_FALSE: success.
        if stream.started.is_some() {
            stream.played_before = stream.played(speed) as f64;
            stream.started = None;
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<(), HResult> {
        let speed = self.speed;
        let stream = self.stream.as_mut().ok_or(AUDCLNT_E_NOT_INITIALIZED)?;
        if stream.started.is_some() {
            return Err(AUDCLNT_E_NOT_STOPPED);
        }
        // What was queued but never played is gone.
        let unplayed =
            (stream.rendered - stream.played(speed)) as usize * stream.format.block_align();
        stream.rendered = 0;
        stream.played_before = 0.0;
        let mut state = lock(&self.state);
        let capture = state
            .captures
            .entry(self.endpoint.endpoint.id.clone())
            .or_default();
        let keep = capture.bytes.len().saturating_sub(unplayed);
        capture.bytes.truncate(keep);
        Ok(())
    }

    fn position_frames(&self) -> Result<u64, HResult> {
        Ok(self.stream()?.played(self.speed))
    }
}

impl Drop for FakeClient {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        if state.holders.get(&self.endpoint.endpoint.id) == Some(&self.serial) {
            state.holders.remove(&self.endpoint.endpoint.id);
        }
    }
}
