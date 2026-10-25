//! WASAPI output on Windows.
//!
//! Exclusive mode bypasses the Windows audio engine: the stream's format
//! becomes the device's format, nothing is mixed in and nothing is
//! resampled. Shared mode goes through the engine, which converts to its
//! mix format (`AUTOCONVERTPCM`).

use std::io;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_UNSUPPORTED_FORMAT,
    AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    DEVICE_STATE_ACTIVE, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0, eConsole,
    eRender,
};
use windows::Win32::Media::KernelStreaming::KSDATAFORMAT_SUBTYPE_PCM;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::PCWSTR;

use crate::{
    Backend, DeviceCaps, DeviceInfo, Driver, STANDARD_RATES, SampleFormat, Sink, StreamFormat,
};

const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SPEAKER_STEREO: u32 = 0x3;
const SPEAKER_MONO: u32 = 0x4;
/// 100-nanosecond units per second.
const HNS_PER_SECOND: i64 = 10_000_000;

pub struct WasapiBackend;

fn com_init() {
    // SAFETY: initialising COM for this thread; repeated calls on the same
    // thread are harmless (S_FALSE or RPC_E_CHANGED_MODE are ignored).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn to_io(error: windows::core::Error, context: &str) -> io::Error {
    let code = error.code();
    let kind = if code == AUDCLNT_E_DEVICE_IN_USE {
        io::ErrorKind::ResourceBusy
    } else if code == AUDCLNT_E_UNSUPPORTED_FORMAT {
        io::ErrorKind::Unsupported
    } else {
        io::ErrorKind::Other
    };
    let message = if code == AUDCLNT_E_DEVICE_IN_USE {
        format!("{context}: the device is in use by another program in exclusive mode")
    } else {
        format!("{context}: {}", error.message())
    };
    io::Error::new(kind, message)
}

fn wave_format(format: StreamFormat) -> WAVEFORMATEXTENSIBLE {
    let container_bits = (format.sample_format.bytes() * 8) as u16;
    let block_align = format.sample_format.bytes() as u16 * u16::from(format.channels);
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: u16::from(format.channels),
            nSamplesPerSec: format.sample_rate,
            nAvgBytesPerSec: format.sample_rate * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: container_bits,
            cbSize: (std::mem::size_of::<WAVEFORMATEXTENSIBLE>()
                - std::mem::size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: u16::from(format.sample_format.bits()),
        },
        dwChannelMask: if format.channels == 1 {
            SPEAKER_MONO
        } else {
            SPEAKER_STEREO
        },
        SubFormat: KSDATAFORMAT_SUBTYPE_PCM,
    }
}

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    // SAFETY: COM is initialised on this thread by the caller.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
    // SAFETY: GetId returns a CoTaskMem string, freed after copying.
    unsafe {
        let id = device.GetId()?;
        let text = id.to_string().unwrap_or_default();
        CoTaskMemFree(Some(id.0 as *const _));
        Ok(text)
    }
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: the property store is read-only and released on drop.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let text = value.to_string();
        (!text.is_empty()).then_some(text)
    }
}

fn activate(device: &IMMDevice) -> windows::core::Result<IAudioClient> {
    // SAFETY: activating an interface on a live endpoint.
    unsafe { device.Activate(CLSCTX_ALL, None) }
}

const PROBE_FORMATS: [SampleFormat; 4] = [
    SampleFormat::S16,
    SampleFormat::S24Packed,
    SampleFormat::S24In32,
    SampleFormat::S32,
];

fn probe(device: &IMMDevice) -> io::Result<DeviceCaps> {
    let client = activate(device).map_err(|error| to_io(error, "WASAPI"))?;
    let mut rates = Vec::new();
    let mut formats = Vec::new();
    for rate in STANDARD_RATES {
        for sample_format in PROBE_FORMATS {
            let wave = wave_format(StreamFormat {
                sample_rate: rate,
                sample_format,
                channels: 2,
            });
            // SAFETY: the format struct outlives the call; exclusive mode
            // does not return a closest match.
            let supported = unsafe {
                client.IsFormatSupported(
                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                    (&raw const wave).cast::<WAVEFORMATEX>(),
                    None,
                )
            }
            .is_ok();
            if supported {
                if !rates.contains(&rate) {
                    rates.push(rate);
                }
                if !formats.contains(&sample_format) {
                    formats.push(sample_format);
                }
            }
        }
    }
    Ok(DeviceCaps {
        sample_rates: rates,
        sample_formats: formats,
        min_channels: 2,
        max_channels: 2,
        // DSD reaches WASAPI only as DoP, which cannot be probed.
        dsd_formats: Vec::new(),
        dsd_rates: Vec::new(),
    })
}

fn find_device(id: &str) -> io::Result<IMMDevice> {
    let endpoint = id
        .strip_prefix("wasapi:")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not a WASAPI device id"))?;
    let enumerator = enumerator().map_err(|error| to_io(error, "WASAPI"))?;
    let wide: Vec<u16> = endpoint.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
        .map_err(|error| to_io(error, "WASAPI device"))
}

impl Backend for WasapiBackend {
    fn driver(&self) -> Driver {
        Driver::Wasapi
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        com_init();
        let Ok(enumerator) = enumerator() else {
            return Vec::new();
        };
        // SAFETY: enumerating live render endpoints.
        let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .ok()
            .and_then(|device| device_id(&device).ok());
        // SAFETY: as above.
        let Ok(collection) =
            (unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) })
        else {
            return Vec::new();
        };
        // SAFETY: as above.
        let count = unsafe { collection.GetCount() }.unwrap_or(0);
        let mut devices = Vec::new();
        for index in 0..count {
            // SAFETY: `index` is below the collection's count.
            let Ok(device) = (unsafe { collection.Item(index) }) else {
                continue;
            };
            let Ok(endpoint) = device_id(&device) else {
                continue;
            };
            let (caps, probe_error) = match probe(&device) {
                Ok(caps) => (Some(caps), None),
                Err(error) => (None, Some(error.to_string())),
            };
            devices.push(DeviceInfo {
                id: format!("wasapi:{endpoint}"),
                name: friendly_name(&device).unwrap_or_else(|| "Audio device".into()),
                detail: "WASAPI endpoint".into(),
                driver: Driver::Wasapi,
                exclusive_capable: true,
                caps,
                probe_error,
                is_default: default_id.as_deref() == Some(endpoint.as_str()),
                hardware: None,
                address: None,
            });
        }
        devices
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        com_init();
        let device = find_device(device_id)?;
        let wave = wave_format(format);
        let pointer = (&raw const wave).cast::<WAVEFORMATEX>();
        // SAFETY: CreateEventW with default security and no name.
        let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
            .map_err(|error| to_io(error, "WASAPI event"))?;
        let opened = if exclusive {
            open_exclusive(&device, pointer, event)
        } else {
            open_shared(&device, pointer, event)
        };
        let (client, buffer_frames) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                // SAFETY: the event was created above and is not used again.
                unsafe {
                    let _ = CloseHandle(event);
                }
                return Err(error);
            }
        };
        // SAFETY: the client is initialised.
        let render: IAudioRenderClient =
            unsafe { client.GetService() }.map_err(|error| to_io(error, "WASAPI render client"))?;
        Ok(Box::new(WasapiSink {
            client,
            render,
            event,
            exclusive,
            buffer_frames,
            frame_bytes: format.frame_bytes(),
            pending: Vec::new(),
            started: false,
        }))
    }
}

fn open_exclusive(
    device: &IMMDevice,
    format: *const WAVEFORMATEX,
    event: HANDLE,
) -> io::Result<(IAudioClient, u32)> {
    let client = activate(device).map_err(|error| to_io(error, "WASAPI"))?;
    let mut default_period = 0_i64;
    // SAFETY: valid out-parameter.
    unsafe { client.GetDevicePeriod(Some(&mut default_period), None) }
        .map_err(|error| to_io(error, "WASAPI device period"))?;
    let mut period = default_period.max(HNS_PER_SECOND / 100);
    // SAFETY: `format` points at a live WAVEFORMATEXTENSIBLE.
    let mut result = unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            period,
            period,
            format,
            None,
        )
    };
    let mut client = client;
    if let Err(error) = &result
        && error.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED
    {
        // Align the period to the buffer size the device asked for, on a
        // fresh client as WASAPI requires.
        // SAFETY: GetBufferSize is valid after this failure.
        let frames = unsafe { client.GetBufferSize() }.map_err(|error| to_io(error, "WASAPI"))?;
        // SAFETY: `format` is valid.
        let rate = i64::from(unsafe { (*format).nSamplesPerSec });
        period = (HNS_PER_SECOND * i64::from(frames) + rate / 2) / rate;
        client = activate(device).map_err(|error| to_io(error, "WASAPI"))?;
        // SAFETY: as above.
        result = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                period,
                period,
                format,
                None,
            )
        };
    }
    result.map_err(|error| to_io(error, "WASAPI Exclusive Mode"))?;
    // SAFETY: the client is initialised and `event` is a live handle.
    unsafe { client.SetEventHandle(event) }.map_err(|error| to_io(error, "WASAPI"))?;
    // SAFETY: as above.
    let frames = unsafe { client.GetBufferSize() }.map_err(|error| to_io(error, "WASAPI"))?;
    Ok((client, frames))
}

fn open_shared(
    device: &IMMDevice,
    format: *const WAVEFORMATEX,
    event: HANDLE,
) -> io::Result<(IAudioClient, u32)> {
    let client = activate(device).map_err(|error| to_io(error, "WASAPI"))?;
    // SAFETY: `format` points at a live WAVEFORMATEXTENSIBLE.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            HNS_PER_SECOND / 5,
            0,
            format,
            None,
        )
    }
    .map_err(|error| to_io(error, "WASAPI Shared Mode"))?;
    // SAFETY: the client is initialised and `event` is a live handle.
    unsafe { client.SetEventHandle(event) }.map_err(|error| to_io(error, "WASAPI"))?;
    // SAFETY: as above.
    let frames = unsafe { client.GetBufferSize() }.map_err(|error| to_io(error, "WASAPI"))?;
    Ok((client, frames))
}

struct WasapiSink {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: HANDLE,
    exclusive: bool,
    buffer_frames: u32,
    frame_bytes: usize,
    pending: Vec<u8>,
    started: bool,
}

// SAFETY: the COM objects live in the multithreaded apartment and are
// only used from the playback thread that owns the sink.
unsafe impl Send for WasapiSink {}

impl WasapiSink {
    fn padding(&self) -> u32 {
        // SAFETY: the client is initialised.
        unsafe { self.client.GetCurrentPadding() }.unwrap_or(0)
    }

    /// Copies up to `frames` frames from `pending` into the device buffer.
    fn submit(&mut self, frames: u32) -> io::Result<()> {
        let bytes = frames as usize * self.frame_bytes;
        // SAFETY: GetBuffer returns room for `frames` frames, which is
        // filled completely before ReleaseBuffer.
        unsafe {
            let buffer = self
                .render
                .GetBuffer(frames)
                .map_err(|error| to_io(error, "WASAPI buffer"))?;
            let available = bytes.min(self.pending.len());
            std::ptr::copy_nonoverlapping(self.pending.as_ptr(), buffer, available);
            if available < bytes {
                std::ptr::write_bytes(buffer.add(available), 0, bytes - available);
            }
            self.render
                .ReleaseBuffer(frames, 0)
                .map_err(|error| to_io(error, "WASAPI buffer"))?;
            self.pending.drain(..available);
        }
        Ok(())
    }

    fn wait(&self) -> io::Result<()> {
        // SAFETY: the event handle is live.
        let result = unsafe { WaitForSingleObject(self.event, 2_000) };
        if result == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the audio device stopped responding",
            ))
        }
    }

    fn start(&mut self) -> io::Result<()> {
        if !self.started {
            // SAFETY: the client is initialised and pre-filled.
            unsafe { self.client.Start() }.map_err(|error| to_io(error, "WASAPI start"))?;
            self.started = true;
        }
        Ok(())
    }

    /// Writes whatever fits, waiting for the device as needed. With
    /// `all`, pads the tail with silence so nothing is left pending.
    fn pump(&mut self, all: bool) -> io::Result<()> {
        let period_bytes = self.buffer_frames as usize * self.frame_bytes;
        loop {
            if self.exclusive {
                // Exclusive event mode: every event wants a whole buffer.
                if self.pending.len() < period_bytes && !(all && !self.pending.is_empty()) {
                    return Ok(());
                }
                if self.started {
                    self.wait()?;
                }
                self.submit(self.buffer_frames)?;
                self.start()?;
            } else {
                if self.pending.len() < self.frame_bytes {
                    return Ok(());
                }
                let room = self.buffer_frames.saturating_sub(self.padding());
                if room == 0 {
                    self.start()?;
                    self.wait()?;
                    continue;
                }
                let frames = room.min((self.pending.len() / self.frame_bytes) as u32);
                self.submit(frames)?;
                if self.padding() * 2 >= self.buffer_frames {
                    self.start()?;
                }
            }
        }
    }
}

impl Sink for WasapiSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.pending.extend_from_slice(data);
        self.pump(false)
    }

    fn delay_frames(&mut self) -> u64 {
        u64::from(self.padding()) + (self.pending.len() / self.frame_bytes.max(1)) as u64
    }

    fn flush(&mut self) -> io::Result<()> {
        // SAFETY: the client is initialised; Reset requires it stopped.
        unsafe {
            let _ = self.client.Stop();
            let _ = self.client.Reset();
        }
        self.pending.clear();
        self.started = false;
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        self.pump(true)?;
        self.start()?;
        for _ in 0..200 {
            if self.padding() == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.flush()
    }
}

impl Drop for WasapiSink {
    fn drop(&mut self) {
        // SAFETY: stopping the stream and closing the event created in
        // `open`, once.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}
