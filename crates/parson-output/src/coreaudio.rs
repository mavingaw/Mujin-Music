//! CoreAudio HAL output on macOS.
//!
//! Exclusive mode takes the device in hog mode, so no other program can
//! play through it, and switches its physical format to the track's
//! sample rate and integer bit depth. The HAL then converts the IOProc's
//! 32-bit float samples back to that integer format; float32 holds every
//! 16- and 24-bit value exactly, so the DAC receives the file's samples.
//! Shared mode only switches the nominal rate, without hog mode.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_void};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    Backend, DeviceCaps, DeviceInfo, Driver, STANDARD_RATES, SampleFormat, Sink, StreamFormat,
};

type ObjectId = u32;
type OsStatus = i32;
type CfStringRef = *const c_void;
type IoProcId = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct StreamDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ValueRange {
    minimum: f64,
    maximum: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RangedDescription {
    format: StreamDescription,
    rate_range: ValueRange,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

type IoProc = unsafe extern "C" fn(
    ObjectId,
    *const c_void,
    *const AudioBufferList,
    *const c_void,
    *mut AudioBufferList,
    *const c_void,
    *mut c_void,
) -> OsStatus;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: ObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> OsStatus;
    fn AudioObjectGetPropertyData(
        object: ObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectSetPropertyData(
        object: ObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OsStatus;
    fn AudioDeviceCreateIOProcID(
        device: ObjectId,
        procedure: IoProc,
        client: *mut c_void,
        id: *mut IoProcId,
    ) -> OsStatus;
    fn AudioDeviceDestroyIOProcID(device: ObjectId, id: IoProcId) -> OsStatus;
    fn AudioDeviceStart(device: ObjectId, id: IoProcId) -> OsStatus;
    fn AudioDeviceStop(device: ObjectId, id: IoProcId) -> OsStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetCString(
        string: CfStringRef,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> u8;
    fn CFRelease(object: *const c_void);
}

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const SYSTEM_OBJECT: ObjectId = 1;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SCOPE_OUTPUT: u32 = fourcc(b"outp");
const ELEMENT_MAIN: u32 = 0;
const HARDWARE_DEVICES: u32 = fourcc(b"dev#");
const HARDWARE_DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
const OBJECT_NAME: u32 = fourcc(b"lnam");
const DEVICE_UID: u32 = fourcc(b"uid ");
const DEVICE_STREAMS: u32 = fourcc(b"stm#");
const DEVICE_HOG_MODE: u32 = fourcc(b"oink");
const DEVICE_NOMINAL_RATE: u32 = fourcc(b"nsrt");
const DEVICE_TRANSPORT: u32 = fourcc(b"tran");
const DEVICE_LATENCY: u32 = fourcc(b"ltnc");
const STREAM_AVAILABLE_PHYSICAL: u32 = fourcc(b"pft#");
const STREAM_PHYSICAL_FORMAT: u32 = fourcc(b"pft ");
const FORMAT_LINEAR_PCM: u32 = fourcc(b"lpcm");
const FLAG_FLOAT: u32 = 1;
const FLAG_SIGNED_INTEGER: u32 = 4;
const UTF8: u32 = 0x0800_0100;

fn address(selector: u32, scope: u32) -> PropertyAddress {
    PropertyAddress {
        selector,
        scope,
        element: ELEMENT_MAIN,
    }
}

fn status(code: OsStatus, context: &str) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        let bytes = code.to_be_bytes();
        let text = if bytes.iter().all(u8::is_ascii_graphic) {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            code.to_string()
        };
        Err(io::Error::other(format!(
            "{context}: CoreAudio error {text}"
        )))
    }
}

fn get_array<T: Copy + Default>(object: ObjectId, address: PropertyAddress) -> io::Result<Vec<T>> {
    let mut size = 0_u32;
    // SAFETY: valid address and out-parameter.
    status(
        unsafe { AudioObjectGetPropertyDataSize(object, &address, 0, std::ptr::null(), &mut size) },
        "property size",
    )?;
    let mut values = vec![T::default(); size as usize / std::mem::size_of::<T>()];
    // SAFETY: `values` has room for `size` bytes.
    status(
        unsafe {
            AudioObjectGetPropertyData(
                object,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                values.as_mut_ptr().cast(),
            )
        },
        "property data",
    )?;
    values.truncate(size as usize / std::mem::size_of::<T>());
    Ok(values)
}

fn get_value<T: Copy + Default>(object: ObjectId, address: PropertyAddress) -> io::Result<T> {
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: `value` has room for `size` bytes.
    status(
        unsafe {
            AudioObjectGetPropertyData(
                object,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                (&raw mut value).cast(),
            )
        },
        "property",
    )?;
    Ok(value)
}

fn set_value<T>(object: ObjectId, address: PropertyAddress, value: &T) -> io::Result<()> {
    // SAFETY: `value` is a live `T` of the size given.
    status(
        unsafe {
            AudioObjectSetPropertyData(
                object,
                &address,
                0,
                std::ptr::null(),
                std::mem::size_of::<T>() as u32,
                (value as *const T).cast(),
            )
        },
        "set property",
    )
}

fn get_string(object: ObjectId, selector: u32) -> Option<String> {
    let reference: CfStringRef =
        get_value::<usize>(object, address(selector, SCOPE_GLOBAL)).ok()? as CfStringRef;
    if reference.is_null() {
        return None;
    }
    let mut buffer = [0 as c_char; 512];
    // SAFETY: the buffer is 512 bytes; the string is released once.
    let ok = unsafe {
        let ok = CFStringGetCString(reference, buffer.as_mut_ptr(), buffer.len() as isize, UTF8);
        CFRelease(reference);
        ok
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: CFStringGetCString wrote a NUL-terminated string.
    Some(
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn output_stream(device: ObjectId) -> Option<ObjectId> {
    get_array::<ObjectId>(device, address(DEVICE_STREAMS, SCOPE_OUTPUT))
        .ok()?
        .first()
        .copied()
}

fn physical_formats(stream: ObjectId) -> io::Result<Vec<RangedDescription>> {
    get_array(stream, address(STREAM_AVAILABLE_PHYSICAL, SCOPE_GLOBAL))
}

fn integer_depth(format: &StreamDescription) -> Option<u8> {
    if format.format_id != FORMAT_LINEAR_PCM
        || format.format_flags & FLAG_FLOAT != 0
        || format.format_flags & FLAG_SIGNED_INTEGER == 0
    {
        return None;
    }
    match format.bits_per_channel {
        16 => Some(16),
        24 => Some(24),
        _ => None,
    }
}

fn caps_from(formats: &[RangedDescription]) -> DeviceCaps {
    let mut rates = Vec::new();
    let mut sample_formats = Vec::new();
    let mut min_channels = u8::MAX;
    let mut max_channels = 0;
    for ranged in formats {
        let Some(bits) = integer_depth(&ranged.format) else {
            continue;
        };
        for rate in STANDARD_RATES {
            let rate_f = f64::from(rate);
            if rate_f >= ranged.rate_range.minimum - 0.5
                && rate_f <= ranged.rate_range.maximum + 0.5
                && !rates.contains(&rate)
            {
                rates.push(rate);
            }
        }
        // The HAL takes float32 from the IOProc; float32 holds 24 bits
        // exactly, so 16 and 24-bit physical formats are bit-perfect.
        let format = if bits == 16 {
            SampleFormat::S16
        } else {
            SampleFormat::S24Packed
        };
        if !sample_formats.contains(&format) {
            sample_formats.push(format);
        }
        let channels = ranged.format.channels_per_frame.clamp(1, 255) as u8;
        min_channels = min_channels.min(channels);
        max_channels = max_channels.max(channels);
    }
    rates.sort_unstable();
    sample_formats.sort_unstable();
    DeviceCaps {
        sample_rates: rates,
        sample_formats,
        min_channels: if max_channels == 0 { 2 } else { min_channels },
        max_channels: if max_channels == 0 { 2 } else { max_channels },
    }
}

fn transport_label(device: ObjectId) -> &'static str {
    match get_value::<u32>(device, address(DEVICE_TRANSPORT, SCOPE_GLOBAL)).unwrap_or(0) {
        code if code == fourcc(b"usb ") => "USB",
        code if code == fourcc(b"bltn") => "Built-in",
        code if code == fourcc(b"hdmi") => "HDMI",
        code if code == fourcc(b"dprt") => "DisplayPort",
        code if code == fourcc(b"blue") || code == fourcc(b"blea") => "Bluetooth",
        code if code == fourcc(b"airp") => "AirPlay",
        code if code == fourcc(b"1394") => "FireWire",
        code if code == fourcc(b"thun") => "Thunderbolt",
        code if code == fourcc(b"virt") => "Virtual",
        _ => "Audio device",
    }
}

fn output_devices() -> Vec<ObjectId> {
    get_array::<ObjectId>(SYSTEM_OBJECT, address(HARDWARE_DEVICES, SCOPE_GLOBAL))
        .unwrap_or_default()
        .into_iter()
        .filter(|device| output_stream(*device).is_some())
        .collect()
}

fn find_device(id: &str) -> io::Result<ObjectId> {
    let uid = id
        .strip_prefix("coreaudio:")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not a CoreAudio device id"))?;
    output_devices()
        .into_iter()
        .find(|device| get_string(*device, DEVICE_UID).as_deref() == Some(uid))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the device is not connected"))
}

pub struct CoreAudioBackend;

impl Backend for CoreAudioBackend {
    fn driver(&self) -> Driver {
        Driver::CoreAudio
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let default = get_value::<ObjectId>(
            SYSTEM_OBJECT,
            address(HARDWARE_DEFAULT_OUTPUT, SCOPE_GLOBAL),
        )
        .ok();
        output_devices()
            .into_iter()
            .filter_map(|device| {
                let uid = get_string(device, DEVICE_UID)?;
                let stream = output_stream(device)?;
                let (caps, probe_error) = match physical_formats(stream) {
                    Ok(formats) => (Some(caps_from(&formats)), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                let hog: i32 =
                    get_value(device, address(DEVICE_HOG_MODE, SCOPE_GLOBAL)).unwrap_or(-1);
                let busy = hog != -1 && hog != std::process::id() as i32;
                Some(DeviceInfo {
                    id: format!("coreaudio:{uid}"),
                    name: get_string(device, OBJECT_NAME).unwrap_or_else(|| "Audio device".into()),
                    detail: transport_label(device).into(),
                    driver: Driver::CoreAudio,
                    exclusive_capable: true,
                    caps,
                    probe_error: busy
                        .then(|| "The device is in exclusive use by another program".to_string())
                        .or(probe_error),
                    is_default: default == Some(device),
                })
            })
            .collect()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        let device = find_device(device_id)?;
        let stream = output_stream(device)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the device has no output"))?;
        let original_physical: Option<StreamDescription> =
            get_value(stream, address(STREAM_PHYSICAL_FORMAT, SCOPE_GLOBAL)).ok();
        let original_rate: Option<f64> =
            get_value(device, address(DEVICE_NOMINAL_RATE, SCOPE_GLOBAL)).ok();
        let mut hogged = false;
        if exclusive {
            let pid = std::process::id() as i32;
            set_value(device, address(DEVICE_HOG_MODE, SCOPE_GLOBAL), &pid)?;
            let owner: i32 = get_value(device, address(DEVICE_HOG_MODE, SCOPE_GLOBAL))?;
            if owner != pid {
                return Err(io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    "the device is in exclusive use by another program",
                ));
            }
            hogged = true;
            let bits = u32::from(format.sample_format.bits());
            let wanted = physical_formats(stream)?
                .into_iter()
                .map(|ranged| ranged.format)
                .find(|candidate| {
                    integer_depth(candidate).map(u32::from) == Some(bits)
                        && candidate.channels_per_frame == u32::from(format.channels)
                })
                .map(|mut candidate| {
                    candidate.sample_rate = f64::from(format.sample_rate);
                    candidate
                });
            let result = match wanted {
                Some(physical) => set_value(
                    stream,
                    address(STREAM_PHYSICAL_FORMAT, SCOPE_GLOBAL),
                    &physical,
                ),
                None => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the device has no integer format of this bit depth",
                )),
            };
            if let Err(error) = result {
                release_hog(device);
                return Err(error);
            }
        } else {
            set_value(
                device,
                address(DEVICE_NOMINAL_RATE, SCOPE_GLOBAL),
                &f64::from(format.sample_rate),
            )?;
        }
        let latency: u32 = get_value(device, address(DEVICE_LATENCY, SCOPE_OUTPUT)).unwrap_or(0);
        let shared = Arc::new(Shared {
            samples: Mutex::new(VecDeque::new()),
            channels: usize::from(format.channels),
        });
        let client = Arc::into_raw(Arc::clone(&shared)) as *mut c_void;
        let mut proc_id: IoProcId = std::ptr::null_mut();
        // SAFETY: `client` is an Arc reference released in Drop after the
        // IOProc is destroyed.
        let created = unsafe { AudioDeviceCreateIOProcID(device, render, client, &mut proc_id) };
        let started = status(created, "IOProc").and_then(|()| {
            // SAFETY: the IOProc was created above.
            status(unsafe { AudioDeviceStart(device, proc_id) }, "start")
        });
        if let Err(error) = started {
            // SAFETY: undoing the setup above.
            unsafe {
                if !proc_id.is_null() {
                    AudioDeviceDestroyIOProcID(device, proc_id);
                }
                drop(Arc::from_raw(client as *const Shared));
            }
            if hogged {
                release_hog(device);
            }
            return Err(error);
        }
        Ok(Box::new(CoreAudioSink {
            device,
            stream,
            proc_id,
            client,
            shared,
            format,
            hogged,
            original_physical,
            original_rate,
            latency_frames: u64::from(latency),
        }))
    }
}

fn release_hog(device: ObjectId) {
    let free: i32 = -1;
    let _ = set_value(device, address(DEVICE_HOG_MODE, SCOPE_GLOBAL), &free);
}

struct Shared {
    samples: Mutex<VecDeque<f32>>,
    channels: usize,
}

/// Fills the device's buffers from the queue; silence on underrun. It
/// never blocks: if the writer holds the lock, this cycle is silent.
unsafe extern "C" fn render(
    _device: ObjectId,
    _now: *const c_void,
    _input: *const AudioBufferList,
    _input_time: *const c_void,
    output: *mut AudioBufferList,
    _output_time: *const c_void,
    client: *mut c_void,
) -> OsStatus {
    // SAFETY: `client` is the Arc<Shared> registered with the IOProc, and
    // `output` is the HAL's buffer list for this cycle.
    unsafe {
        let shared = &*(client as *const Shared);
        let list = &mut *output;
        let buffers =
            std::slice::from_raw_parts_mut(list.buffers.as_mut_ptr(), list.count as usize);
        let mut queue = shared.samples.try_lock().ok();
        if buffers.len() == 1 {
            let buffer = &mut buffers[0];
            let device_channels = buffer.channels.max(1) as usize;
            let out = std::slice::from_raw_parts_mut(
                buffer.data.cast::<f32>(),
                buffer.byte_size as usize / 4,
            );
            for frame in out.chunks_mut(device_channels) {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = if channel < shared.channels {
                        queue
                            .as_mut()
                            .and_then(|queue| queue.pop_front())
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                }
            }
        } else {
            // Non-interleaved: one buffer per channel.
            let frames = buffers
                .first()
                .map_or(0, |buffer| buffer.byte_size as usize / 4);
            for frame in 0..frames {
                for (channel, buffer) in buffers.iter_mut().enumerate() {
                    let out = buffer.data.cast::<f32>().add(frame);
                    *out = if channel < shared.channels {
                        queue
                            .as_mut()
                            .and_then(|queue| queue.pop_front())
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                }
            }
        }
    }
    0
}

struct CoreAudioSink {
    device: ObjectId,
    stream: ObjectId,
    proc_id: IoProcId,
    client: *mut c_void,
    shared: Arc<Shared>,
    format: StreamFormat,
    hogged: bool,
    original_physical: Option<StreamDescription>,
    original_rate: Option<f64>,
    latency_frames: u64,
}

// SAFETY: the raw pointers are owned by this sink and only released in
// Drop; the queue they share is behind a mutex.
unsafe impl Send for CoreAudioSink {}

impl CoreAudioSink {
    fn queued_frames(&self) -> usize {
        self.shared
            .samples
            .lock()
            .map(|queue| queue.len() / self.shared.channels.max(1))
            .unwrap_or(0)
    }
}

const SCALE_16: f32 = 1.0 / 32_768.0;
const SCALE_24: f32 = 1.0 / 8_388_608.0;

impl Sink for CoreAudioSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let samples: Vec<f32> = match self.format.sample_format {
            SampleFormat::S16 => data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| f32::from(i16::from_le_bytes(*s)) * SCALE_16)
                .collect(),
            SampleFormat::S24Packed => data
                .as_chunks::<3>()
                .0
                .iter()
                .map(|s| (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 * SCALE_24)
                .collect(),
            SampleFormat::S24In32 | SampleFormat::S32 => data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|s| (i32::from_le_bytes(*s) >> 8) as f32 * SCALE_24)
                .collect(),
        };
        let limit = self.format.sample_rate as usize * self.shared.channels / 4;
        let mut offset = 0;
        while offset < samples.len() {
            let room = {
                let queue = self
                    .shared
                    .samples
                    .lock()
                    .map_err(|_| io::Error::other("audio queue poisoned"))?;
                limit.saturating_sub(queue.len())
            };
            if room == 0 {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let end = (offset + room).min(samples.len());
            if let Ok(mut queue) = self.shared.samples.lock() {
                queue.extend(&samples[offset..end]);
            }
            offset = end;
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        self.queued_frames() as u64 + self.latency_frames
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Ok(mut queue) = self.shared.samples.lock() {
            queue.clear();
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        for _ in 0..400 {
            if self.queued_frames() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(
            self.latency_frames * 1000 / u64::from(self.format.sample_rate.max(1)),
        ));
        Ok(())
    }
}

impl Drop for CoreAudioSink {
    fn drop(&mut self) {
        // SAFETY: stopping and destroying the IOProc created in `open`
        // before releasing the Arc it reads.
        unsafe {
            AudioDeviceStop(self.device, self.proc_id);
            AudioDeviceDestroyIOProcID(self.device, self.proc_id);
            drop(Arc::from_raw(self.client as *const Shared));
        }
        if let Some(original) = self.original_physical {
            let _ = set_value(
                self.stream,
                address(STREAM_PHYSICAL_FORMAT, SCOPE_GLOBAL),
                &original,
            );
        } else if let Some(rate) = self.original_rate {
            let _ = set_value(
                self.device,
                address(DEVICE_NOMINAL_RATE, SCOPE_GLOBAL),
                &rate,
            );
        }
        if self.hogged {
            release_hog(self.device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranged(bits: u32, min: f64, max: f64) -> RangedDescription {
        RangedDescription {
            format: StreamDescription {
                sample_rate: min,
                format_id: FORMAT_LINEAR_PCM,
                format_flags: FLAG_SIGNED_INTEGER,
                bits_per_channel: bits,
                channels_per_frame: 2,
                ..StreamDescription::default()
            },
            rate_range: ValueRange {
                minimum: min,
                maximum: max,
            },
        }
    }

    #[test]
    fn physical_formats_become_caps() {
        let caps = caps_from(&[
            ranged(16, 44_100.0, 48_000.0),
            ranged(24, 44_100.0, 192_000.0),
        ]);
        assert_eq!(
            caps.sample_rates,
            vec![44_100, 48_000, 88_200, 96_000, 176_400, 192_000]
        );
        assert_eq!(
            caps.sample_formats,
            vec![SampleFormat::S16, SampleFormat::S24Packed]
        );
        assert_eq!(caps.max_channels, 2);
    }

    #[test]
    fn fourcc_matches_coreaudio() {
        assert_eq!(fourcc(b"lpcm"), 0x6c70_636d);
    }
}
