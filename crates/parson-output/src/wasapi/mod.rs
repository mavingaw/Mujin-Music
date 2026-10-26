//! WASAPI output for Windows, in exclusive mode: the DAC runs at the
//! file's own sample rate and bit depth and the samples reach it with no
//! Windows mixer, resampler or volume in between (bit-perfect). Shared mode
//! (when exclusive is off for a zone) goes through the Windows mixer with
//! its own format conversion, which is not bit-perfect.
//!
//! The logic here (format probing, the event-driven buffer handshake with
//! its alignment retry, pausing, draining, hardware volume in dB) works on
//! the [`WasapiApi`] trait. On Windows it is backed by the real COM API
//! ([`com`]); everywhere it is also backed by [`fake::FakeWasapi`], which
//! enforces WASAPI's documented rules (exact formats only in exclusive
//! mode, `AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED` needing a fresh client, whole
//! buffers per event, `AUDCLNT_E_DEVICE_IN_USE` while another app holds the
//! device) so the tests prove the logic on any machine.

#[cfg(windows)]
pub mod com;
pub mod fake;

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    Backend, DeviceCaps, DeviceInfo, Driver, HardwareVolume, STANDARD_RATES, SampleFormat, Sink,
    StreamFormat,
};

pub type HResult = i32;

pub const S_OK: HResult = 0;
pub const S_FALSE: HResult = 1;
pub const E_INVALIDARG: HResult = 0x8007_0057_u32 as i32;
pub const E_NOTFOUND: HResult = 0x8007_0490_u32 as i32;
pub const AUDCLNT_E_NOT_INITIALIZED: HResult = 0x8889_0001_u32 as i32;
pub const AUDCLNT_E_ALREADY_INITIALIZED: HResult = 0x8889_0002_u32 as i32;
pub const AUDCLNT_E_DEVICE_INVALIDATED: HResult = 0x8889_0004_u32 as i32;
pub const AUDCLNT_E_NOT_STOPPED: HResult = 0x8889_0005_u32 as i32;
pub const AUDCLNT_E_BUFFER_TOO_LARGE: HResult = 0x8889_0006_u32 as i32;
pub const AUDCLNT_E_UNSUPPORTED_FORMAT: HResult = 0x8889_0008_u32 as i32;
pub const AUDCLNT_E_DEVICE_IN_USE: HResult = 0x8889_000A_u32 as i32;
pub const AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED: HResult = 0x8889_000E_u32 as i32;
pub const AUDCLNT_E_INVALID_SIZE: HResult = 0x8889_0011_u32 as i32;
pub const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED: HResult = 0x8889_0019_u32 as i32;

/// Rates probed on every endpoint.
const PROBE_RATES: &[u32] = &STANDARD_RATES;

/// Shared-mode buffer, in 100 ns units (200 ms).
const SHARED_BUFFER_HNS: i64 = 2_000_000;
/// How long an exclusive stream may go without asking for audio.
const EVENT_TIMEOUT_MS: u32 = 2_000;

/// What an HRESULT means, for people.
pub fn describe(hr: HResult) -> String {
    let text = match hr {
        AUDCLNT_E_DEVICE_IN_USE => "the device is in use by another app in exclusive mode",
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => {
            "exclusive mode is turned off for this device in Windows sound settings (Properties, Advanced, \"Allow applications to take exclusive control\")"
        }
        AUDCLNT_E_UNSUPPORTED_FORMAT => "the device does not take this format natively",
        AUDCLNT_E_DEVICE_INVALIDATED => "the device was unplugged or disabled",
        E_NOTFOUND => "the device is gone",
        _ => "",
    };
    if text.is_empty() {
        format!("WASAPI error 0x{:08X}", hr as u32)
    } else {
        format!("{text} (0x{:08X})", hr as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShareMode {
    Shared,
    Exclusive,
}

/// A PCM `WAVEFORMATEXTENSIBLE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaveFormat {
    pub sample_rate: u32,
    pub channels: u16,
    /// `wBitsPerSample`: 16, 24 or 32.
    pub container_bits: u16,
    /// `wValidBitsPerSample`: 24 in a 32-bit container on many DACs.
    pub valid_bits: u16,
}

impl WaveFormat {
    pub fn block_align(&self) -> usize {
        usize::from(self.channels) * usize::from(self.container_bits / 8)
    }

    /// The PCM wave format for a stream; `None` for DSD, which WASAPI only
    /// carries as DoP (already PCM by the time it gets here).
    fn of(format: StreamFormat) -> Option<Self> {
        let (container_bits, valid_bits) = match format.sample_format {
            SampleFormat::S16 => (16, 16),
            SampleFormat::S24Packed => (24, 24),
            // 24 valid bits in a 32-bit container, as many USB DACs take.
            SampleFormat::S24In32 => (32, 24),
            SampleFormat::S32 => (32, 32),
            _ => return None,
        };
        Some(Self {
            sample_rate: format.sample_rate,
            channels: u16::from(format.channels),
            container_bits,
            valid_bits,
        })
    }
}

/// One active render endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    /// The MMDevice id (`{0.0.0.00000000}.{guid}`).
    pub id: String,
    /// `PKEY_Device_FriendlyName`, e.g. `Speakers (Topping E30 II)`.
    pub name: String,
    /// `PKEY_DeviceInterface_FriendlyName`, e.g. `Topping E30 II`.
    pub adapter: String,
    /// The shared-mode mix format.
    pub mix: WaveFormat,
}

/// `IAudioEndpointVolume`: the range in dB and whether the hardware does it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndpointVolume {
    pub min_db: f32,
    pub max_db: f32,
    pub step_db: f32,
    /// `ENDPOINT_HARDWARE_SUPPORT_VOLUME`: the DAC's own control. Without
    /// it Windows scales samples in software, which exclusive streams
    /// bypass, so Parson does not offer it as hardware volume.
    pub hardware: bool,
}

/// The parts of `IMMDeviceEnumerator`, `IMMDevice`, `IAudioEndpointVolume`
/// and `IAudioClient::IsFormatSupported` Parson uses.
pub trait WasapiApi: Send + Sync + 'static {
    type Client: WasapiClient;
    fn endpoints(&self) -> Result<Vec<Endpoint>, HResult>;
    /// `S_OK` when the endpoint takes `format` natively in `mode`.
    fn is_format_supported(&self, id: &str, mode: ShareMode, format: &WaveFormat) -> HResult;
    fn endpoint_volume(&self, id: &str) -> Result<EndpointVolume, HResult>;
    fn set_volume_db(&self, id: &str, db: f32) -> Result<(), HResult>;
    fn set_mute(&self, id: &str, muted: bool) -> Result<(), HResult>;
    /// `IMMDevice::Activate(IID_IAudioClient)`: a fresh, uninitialised client.
    fn activate(&self, id: &str) -> Result<Self::Client, HResult>;
}

/// An `IAudioClient` with its `IAudioRenderClient` and `IAudioClock`.
pub trait WasapiClient: Send + 'static {
    /// `GetDevicePeriod`: (default, minimum) in 100 ns units.
    fn device_period(&self) -> Result<(i64, i64), HResult>;
    /// `Initialize`. Event-driven streams get their event handle set here;
    /// shared streams ask Windows to convert the format.
    fn initialize(
        &mut self,
        mode: ShareMode,
        event_driven: bool,
        buffer_hns: i64,
        period_hns: i64,
        format: &WaveFormat,
    ) -> Result<(), HResult>;
    /// `GetBufferSize`, in frames.
    fn buffer_frames(&self) -> Result<u32, HResult>;
    /// `GetCurrentPadding`, in frames.
    fn padding_frames(&self) -> Result<u32, HResult>;
    /// Waits for the buffer event; `false` on timeout.
    fn wait(&self, timeout_ms: u32) -> bool;
    /// `GetBuffer(frames)`, copy `data`, `ReleaseBuffer(frames, 0)`.
    fn render(&mut self, frames: u32, data: &[u8]) -> Result<(), HResult>;
    fn start(&mut self) -> Result<(), HResult>;
    fn stop(&mut self) -> Result<(), HResult>;
    fn reset(&mut self) -> Result<(), HResult>;
    /// `IAudioClock::GetPosition` in frames: what the device has played.
    fn position_frames(&self) -> Result<u64, HResult>;
}

#[derive(Clone, Debug)]
struct Probe {
    info: DeviceInfo,
    endpoint: String,
    /// Exclusive mode is off for the device: only shared streams work.
    shared_only: bool,
}

fn io_error(message: String) -> io::Error {
    io::Error::other(message)
}

pub struct WasapiBackend<A: WasapiApi> {
    api: Arc<A>,
    /// Last good probe per device, reused while Parson holds it.
    probes: Mutex<HashMap<String, Probe>>,
    open: Arc<Mutex<HashSet<String>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A short stable id from an MMDevice id: its endpoint GUID.
pub fn device_id(endpoint: &str) -> String {
    let guid = endpoint.rsplit('.').next().unwrap_or(endpoint);
    let cleaned: String = guid
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    format!("wasapi-{cleaned}")
}

fn rate_text(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    if rate.is_multiple_of(1000) {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
    }
}

impl<A: WasapiApi> WasapiBackend<A> {
    pub fn new(api: A) -> Self {
        Self {
            api: Arc::new(api),
            probes: Mutex::new(HashMap::new()),
            open: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn api(&self) -> &A {
        &self.api
    }

    fn probe(&self, endpoint: &Endpoint) -> Probe {
        let id = device_id(&endpoint.id);
        let candidates: [(SampleFormat, u16, u16); 4] = [
            (SampleFormat::S16, 16, 16),
            (SampleFormat::S24Packed, 24, 24),
            (SampleFormat::S24In32, 32, 24),
            (SampleFormat::S32, 32, 32),
        ];
        let mut rates = Vec::new();
        let mut formats: Vec<SampleFormat> = Vec::new();
        let mut busy = false;
        let mut not_allowed = false;
        let channels = endpoint.mix.channels.clamp(1, 2);
        for rate in PROBE_RATES {
            let mut any = false;
            for (format, container_bits, valid_bits) in candidates {
                let wave = WaveFormat {
                    sample_rate: *rate,
                    channels,
                    container_bits,
                    valid_bits,
                };
                match self
                    .api
                    .is_format_supported(&endpoint.id, ShareMode::Exclusive, &wave)
                {
                    S_OK => {
                        any = true;
                        if !formats.contains(&format) {
                            formats.push(format);
                        }
                    }
                    AUDCLNT_E_DEVICE_IN_USE => busy = true,
                    AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => not_allowed = true,
                    _ => {}
                }
            }
            if any {
                rates.push(*rate);
            }
        }
        let shared_only = rates.is_empty() && not_allowed;
        if shared_only {
            // Windows converts whatever Parson sends to the mix format.
            rates = vec![endpoint.mix.sample_rate];
            formats = vec![
                SampleFormat::S16,
                SampleFormat::S24Packed,
                SampleFormat::S32,
            ];
        }
        // Narrowest first; a packed 24-bit container before 24-in-32.
        formats.sort_by_key(|format| (format.bits(), format.bytes()));
        let hardware_volume = self
            .api
            .endpoint_volume(&endpoint.id)
            .ok()
            .filter(|volume| volume.hardware && volume.max_db > volume.min_db)
            .map(|volume| HardwareVolume {
                min_db: volume.min_db,
                max_db: volume.max_db,
                step_db: if volume.step_db > 0.0 {
                    volume.step_db
                } else {
                    0.5
                },
            });
        let adapter = if endpoint.adapter.is_empty() {
            "Audio device".to_string()
        } else {
            endpoint.adapter.clone()
        };
        let description = if shared_only {
            format!(
                "{adapter} · WASAPI shared only: exclusive mode is turned off for this device in Windows sound settings, so Windows mixes at {} (not bit-perfect)",
                rate_text(endpoint.mix.sample_rate)
            )
        } else {
            format!("{adapter} · WASAPI exclusive")
        };
        let max_channels = u8::try_from(endpoint.mix.channels.max(1)).unwrap_or(u8::MAX);
        Probe {
            info: DeviceInfo {
                id,
                name: endpoint.name.clone(),
                detail: description,
                driver: Driver::Wasapi,
                exclusive_capable: !shared_only,
                caps: Some(DeviceCaps {
                    sample_rates: rates,
                    sample_formats: formats,
                    min_channels: 1,
                    max_channels,
                    // DSD reaches WASAPI only as DoP, which cannot be probed.
                    dsd_formats: Vec::new(),
                    dsd_rates: Vec::new(),
                }),
                probe_error: busy.then(|| describe(AUDCLNT_E_DEVICE_IN_USE)),
                is_default: false,
                hardware: None,
                address: None,
                network: None,
                hardware_volume,
                busy,
            },
            endpoint: endpoint.id.clone(),
            shared_only,
        }
    }

    fn lookup(&self, device_id: &str) -> Option<Probe> {
        if let Some(probe) = lock(&self.probes).get(device_id) {
            return Some(probe.clone());
        }
        self.devices();
        lock(&self.probes).get(device_id).cloned()
    }

    /// `Initialize` in exclusive, event-driven mode at the device's default
    /// period, with the documented retry when the buffer is not aligned:
    /// read the aligned size, release the client, activate a fresh one and
    /// initialise it with the period that size implies.
    fn initialize_exclusive(
        &self,
        endpoint: &str,
        wave: &WaveFormat,
    ) -> Result<A::Client, HResult> {
        let mut client = self.api.activate(endpoint)?;
        let (default_period, minimum_period) = client.device_period()?;
        let period = default_period.max(minimum_period).max(1);
        match client.initialize(ShareMode::Exclusive, true, period, period, wave) {
            Ok(()) => Ok(client),
            Err(AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED) => {
                let frames = client.buffer_frames()?;
                drop(client);
                let aligned =
                    (10_000_000.0 * f64::from(frames) / f64::from(wave.sample_rate) + 0.5) as i64;
                let mut client = self.api.activate(endpoint)?;
                client.initialize(ShareMode::Exclusive, true, aligned, aligned, wave)?;
                Ok(client)
            }
            Err(error) => Err(error),
        }
    }
}

impl<A: WasapiApi> Backend for WasapiBackend<A> {
    fn driver(&self) -> Driver {
        Driver::Wasapi
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let Ok(endpoints) = self.api.endpoints() else {
            return Vec::new();
        };
        let open = lock(&self.open).clone();
        let mut probes = lock(&self.probes);
        let mut devices = Vec::new();
        for endpoint in endpoints {
            let id = device_id(&endpoint.id);
            // Parson holds it: Windows says "in use", so reuse what the
            // device took before.
            if open.contains(&id)
                && let Some(probe) = probes.get(&id)
            {
                devices.push(probe.info.clone());
                continue;
            }
            let probe = self.probe(&endpoint);
            if probe.info.sample_rates().is_empty() {
                if let Some(previous) = probes.get(&id).filter(|_| probe.info.busy) {
                    // Another app holds it exclusively: list what it took
                    // before, marked busy.
                    let mut info = previous.info.clone();
                    info.busy = true;
                    info.probe_error = probe.info.probe_error.clone();
                    devices.push(info);
                }
                continue;
            }
            devices.push(probe.info.clone());
            probes.insert(id, probe);
        }
        devices
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        let probe = self
            .lookup(device_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device unplugged"))?;
        let name = probe.info.name.clone();
        let wave = WaveFormat::of(format).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "{name} takes DSD only as DoP over WASAPI, not as {}",
                    format.sample_format.label()
                ),
            )
        })?;
        let exclusive = exclusive && !probe.shared_only;
        let (client, buffer_frames) = if exclusive {
            let client = self
                .initialize_exclusive(&probe.endpoint, &wave)
                .map_err(|hr| {
                    if hr == AUDCLNT_E_UNSUPPORTED_FORMAT {
                        io::Error::new(
                            io::ErrorKind::Unsupported,
                            format!(
                                "{name} does not take {} Hz {} in exclusive mode",
                                format.sample_rate,
                                format.sample_format.alsa_name()
                            ),
                        )
                    } else {
                        io_error(format!("{name}: {}", describe(hr)))
                    }
                })?;
            let frames = client
                .buffer_frames()
                .map_err(|hr| io_error(format!("{name}: {}", describe(hr))))?;
            (client, frames)
        } else {
            let mut client = self
                .api
                .activate(&probe.endpoint)
                .map_err(|hr| io_error(format!("{name}: {}", describe(hr))))?;
            client
                .initialize(ShareMode::Shared, false, SHARED_BUFFER_HNS, 0, &wave)
                .map_err(|hr| io_error(format!("{name}: {}", describe(hr))))?;
            let frames = client
                .buffer_frames()
                .map_err(|hr| io_error(format!("{name}: {}", describe(hr))))?;
            (client, frames)
        };
        if buffer_frames == 0 {
            return Err(io_error(format!(
                "{name}: the device reported an empty buffer"
            )));
        }
        lock(&self.open).insert(device_id.to_string());
        Ok(Box::new(WasapiStream {
            client,
            format,
            exclusive,
            buffer_frames,
            staging: Vec::new(),
            written: 0,
            running: false,
            paused: false,
            device_id: device_id.to_string(),
            name,
            open: self.open.clone(),
        }))
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> io::Result<()> {
        let probe = self
            .lookup(device_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device unplugged"))?;
        let range = probe.info.hardware_volume.ok_or_else(|| {
            crate::unsupported(&format!("{} has no hardware volume", probe.info.name))
        })?;
        self.api
            .set_volume_db(&probe.endpoint, db.clamp(range.min_db, range.max_db))
            .map_err(|hr| io_error(describe(hr)))
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> io::Result<()> {
        let probe = self
            .lookup(device_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device unplugged"))?;
        self.api
            .set_mute(&probe.endpoint, muted)
            .map_err(|hr| io_error(describe(hr)))
    }
}

/// An open WASAPI stream. Exclusive streams are event-driven: Windows
/// signals when one buffer has played, and Parson hands it exactly one
/// whole buffer back. Shared streams fill whatever room there is.
pub struct WasapiStream<C: WasapiClient> {
    client: C,
    format: StreamFormat,
    exclusive: bool,
    buffer_frames: u32,
    /// Audio not yet given to the device (less than one buffer).
    staging: Vec<u8>,
    /// Frames given to the device since the last reset.
    written: u64,
    running: bool,
    paused: bool,
    device_id: String,
    name: String,
    open: Arc<Mutex<HashSet<String>>>,
}

impl<C: WasapiClient> WasapiStream<C> {
    fn frame_bytes(&self) -> usize {
        self.format.frame_bytes()
    }

    fn error(&self, hr: HResult) -> io::Error {
        io_error(format!("{}: {}", self.name, describe(hr)))
    }

    fn start(&mut self) -> io::Result<()> {
        if !self.running && !self.paused {
            self.client.start().map_err(|hr| self.error(hr))?;
            self.running = true;
        }
        Ok(())
    }

    /// Hands the device one whole buffer (exclusive): the first one before
    /// starting, every later one when the device asks for it.
    fn push_buffer(&mut self, data: &[u8]) -> io::Result<()> {
        if self.written > 0 && !self.client.wait(EVENT_TIMEOUT_MS) {
            return Err(io_error(format!(
                "{} stopped asking for audio (no buffer event for {} s)",
                self.name,
                EVENT_TIMEOUT_MS / 1000
            )));
        }
        self.client
            .render(self.buffer_frames, data)
            .map_err(|hr| self.error(hr))?;
        self.written += u64::from(self.buffer_frames);
        self.start()
    }

    fn played(&self) -> u64 {
        self.client.position_frames().unwrap_or(0).min(self.written)
    }

    fn buffer_duration(&self) -> Duration {
        Duration::from_secs_f64(
            f64::from(self.buffer_frames) / f64::from(self.format.sample_rate.max(1)),
        )
    }
}

impl<C: WasapiClient> Sink for WasapiStream<C> {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let frame_bytes = self.frame_bytes();
        if !data.len().is_multiple_of(frame_bytes) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "partial frame"));
        }
        if self.format.sample_format == SampleFormat::S24In32 {
            // Parson's 24-in-32 words hold the sample in the low 24 bits
            // (ALSA's S24_LE); WASAPI wants the valid bits at the top.
            for word in data.as_chunks::<4>().0 {
                let value = i32::from_le_bytes(*word) << 8;
                self.staging.extend_from_slice(&value.to_le_bytes());
            }
        } else {
            self.staging.extend_from_slice(data);
        }
        if self.exclusive {
            let buffer_bytes = self.buffer_frames as usize * frame_bytes;
            while self.staging.len() >= buffer_bytes {
                let chunk: Vec<u8> = self.staging.drain(..buffer_bytes).collect();
                self.push_buffer(&chunk)?;
            }
            return Ok(());
        }
        let started = Instant::now();
        while !self.staging.is_empty() {
            let padding = self.client.padding_frames().map_err(|hr| self.error(hr))?;
            let room = self.buffer_frames.saturating_sub(padding);
            if room == 0 {
                self.start()?;
                if self.paused || started.elapsed() > Duration::from_secs(5) {
                    return Err(io_error(format!("{} stopped taking audio", self.name)));
                }
                std::thread::sleep((self.buffer_duration() / 8).max(Duration::from_millis(1)));
                continue;
            }
            let frames = room.min((self.staging.len() / frame_bytes) as u32);
            let bytes = frames as usize * frame_bytes;
            self.client
                .render(frames, &self.staging[..bytes])
                .map_err(|hr| self.error(hr))?;
            self.staging.drain(..bytes);
            self.written += u64::from(frames);
        }
        self.start()
    }

    fn delay_frames(&mut self) -> u64 {
        let staged = (self.staging.len() / self.frame_bytes()) as u64;
        self.written.saturating_sub(self.played()) + staged
    }

    fn pause(&mut self, paused: bool) -> io::Result<()> {
        self.paused = paused;
        if paused {
            if self.running {
                self.client.stop().map_err(|hr| self.error(hr))?;
                self.running = false;
            }
        } else if self.written > 0 {
            self.start()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.running {
            let _ = self.client.stop();
            self.running = false;
        }
        let _ = self.client.reset();
        self.staging.clear();
        self.written = 0;
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        let frame_bytes = self.frame_bytes();
        if self.exclusive {
            // The last partial buffer, padded with silence.
            if !self.staging.is_empty() {
                let mut chunk = std::mem::take(&mut self.staging);
                chunk.resize(self.buffer_frames as usize * frame_bytes, 0);
                if let Err(error) = self.push_buffer(&chunk) {
                    let _ = self.flush();
                    return Err(error);
                }
            }
            // An exclusive device replays stale memory if it is not fed, so
            // keep handing it silence until the music has been heard.
            let end = self.written;
            let silence = vec![0_u8; self.buffer_frames as usize * frame_bytes];
            let deadline = Instant::now() + self.buffer_duration() * 4 + Duration::from_secs(2);
            while self.running && self.played() < end && Instant::now() < deadline {
                if self.push_buffer(&silence).is_err() {
                    break;
                }
            }
        } else {
            let deadline = Instant::now()
                + Duration::from_secs_f64(
                    self.written.saturating_sub(self.played()) as f64
                        / f64::from(self.format.sample_rate.max(1)),
                )
                + Duration::from_secs(2);
            if self.start().is_ok() {
                while self.running
                    && self.client.padding_frames().unwrap_or(0) > 0
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        self.flush()
    }
}

impl<C: WasapiClient> Drop for WasapiStream<C> {
    fn drop(&mut self) {
        if self.running {
            let _ = self.client.stop();
        }
        lock(&self.open).remove(&self.device_id);
    }
}

#[cfg(test)]
mod tests;
