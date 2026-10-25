//! ALSA `hw:` output, with libasound loaded at run time.
//!
//! A `hw:` PCM talks to the driver directly: there is no dmix mixer, no
//! rate converter and no format plug in front of it, so a format it accepts
//! reaches the DAC bit for bit and only one program can use it at a time.
//! `plughw:` is the same device with ALSA's conversion plugins enabled,
//! used when exclusive mode is off.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::io;
use std::sync::OnceLock;

use crate::procfs::{self, CapsSource, HardwareInfo, HardwareStatus};
use crate::{
    Backend, DSD_FORMATS, DSD_RATES, DeviceCaps, DeviceInfo, Driver, STANDARD_RATES, SampleFormat,
    Sink, StreamFormat,
};

type Ctl = c_void;
type Pcm = c_void;
type PcmInfo = c_void;
type CardInfo = c_void;
type HwParams = c_void;

const STREAM_PLAYBACK: c_int = 0;
const MODE_NONBLOCK: c_int = 1;
const ACCESS_RW_INTERLEAVED: c_int = 3;
const FORMAT_S16_LE: c_int = 2;
const FORMAT_S24_LE: c_int = 6;
const FORMAT_S32_LE: c_int = 10;
const FORMAT_S24_3LE: c_int = 32;
const FORMAT_DSD_U8: c_int = 48;
const FORMAT_DSD_U16_LE: c_int = 49;
const FORMAT_DSD_U32_LE: c_int = 50;
const FORMAT_DSD_U16_BE: c_int = 51;
const FORMAT_DSD_U32_BE: c_int = 52;
/// Requested device buffer, in microseconds.
const LATENCY_US: c_uint = 200_000;

fn alsa_format(format: SampleFormat) -> c_int {
    match format {
        SampleFormat::S16 => FORMAT_S16_LE,
        SampleFormat::S24Packed => FORMAT_S24_3LE,
        SampleFormat::S24In32 => FORMAT_S24_LE,
        SampleFormat::S32 => FORMAT_S32_LE,
        SampleFormat::DsdU8 => FORMAT_DSD_U8,
        SampleFormat::DsdU16Le => FORMAT_DSD_U16_LE,
        SampleFormat::DsdU16Be => FORMAT_DSD_U16_BE,
        SampleFormat::DsdU32Le => FORMAT_DSD_U32_LE,
        SampleFormat::DsdU32Be => FORMAT_DSD_U32_BE,
    }
}

macro_rules! alsa_api {
    ($($name:ident: fn($($arg:ty),*) -> $ret:ty;)*) => {
        #[allow(non_snake_case)]
        struct Api {
            $($name: unsafe extern "C" fn($($arg),*) -> $ret,)*
        }

        impl Api {
            /// Resolves every symbol, or `None` when libasound is missing.
            fn load() -> Option<Self> {
                // SAFETY: dlopen and dlsym are called with NUL-terminated
                // names; each symbol is transmuted to the signature
                // libasound documents for it.
                unsafe {
                    let library = libc::dlopen(
                        c"libasound.so.2".as_ptr(),
                        libc::RTLD_NOW | libc::RTLD_LOCAL,
                    );
                    if library.is_null() {
                        return None;
                    }
                    Some(Self {
                        $($name: {
                            let symbol = libc::dlsym(
                                library,
                                concat!(stringify!($name), "\0").as_ptr().cast(),
                            );
                            if symbol.is_null() {
                                return None;
                            }
                            std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) -> $ret>(symbol)
                        },)*
                    })
                }
            }
        }
    };
}

alsa_api! {
    snd_card_next: fn(*mut c_int) -> c_int;
    snd_card_get_name: fn(c_int, *mut *mut c_char) -> c_int;
    snd_card_get_longname: fn(c_int, *mut *mut c_char) -> c_int;
    snd_ctl_open: fn(*mut *mut Ctl, *const c_char, c_int) -> c_int;
    snd_ctl_close: fn(*mut Ctl) -> c_int;
    snd_ctl_card_info_malloc: fn(*mut *mut CardInfo) -> c_int;
    snd_ctl_card_info_free: fn(*mut CardInfo) -> ();
    snd_ctl_card_info: fn(*mut Ctl, *mut CardInfo) -> c_int;
    snd_ctl_card_info_get_id: fn(*const CardInfo) -> *const c_char;
    snd_ctl_pcm_next_device: fn(*mut Ctl, *mut c_int) -> c_int;
    snd_ctl_pcm_info: fn(*mut Ctl, *mut PcmInfo) -> c_int;
    snd_pcm_info_malloc: fn(*mut *mut PcmInfo) -> c_int;
    snd_pcm_info_free: fn(*mut PcmInfo) -> ();
    snd_pcm_info_set_device: fn(*mut PcmInfo, c_uint) -> ();
    snd_pcm_info_set_subdevice: fn(*mut PcmInfo, c_uint) -> ();
    snd_pcm_info_set_stream: fn(*mut PcmInfo, c_int) -> ();
    snd_pcm_info_get_name: fn(*const PcmInfo) -> *const c_char;
    snd_pcm_open: fn(*mut *mut Pcm, *const c_char, c_int, c_int) -> c_int;
    snd_pcm_close: fn(*mut Pcm) -> c_int;
    snd_pcm_hw_params_malloc: fn(*mut *mut HwParams) -> c_int;
    snd_pcm_hw_params_free: fn(*mut HwParams) -> ();
    snd_pcm_hw_params_any: fn(*mut Pcm, *mut HwParams) -> c_int;
    snd_pcm_hw_params_test_rate: fn(*mut Pcm, *mut HwParams, c_uint, c_int) -> c_int;
    snd_pcm_hw_params_test_format: fn(*mut Pcm, *mut HwParams, c_int) -> c_int;
    snd_pcm_hw_params_set_format: fn(*mut Pcm, *mut HwParams, c_int) -> c_int;
    snd_pcm_hw_params_copy: fn(*mut HwParams, *const HwParams) -> ();
    snd_pcm_hw_params_get_channels_min: fn(*const HwParams, *mut c_uint) -> c_int;
    snd_pcm_hw_params_get_channels_max: fn(*const HwParams, *mut c_uint) -> c_int;
    snd_pcm_set_params: fn(*mut Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    snd_pcm_writei: fn(*mut Pcm, *const c_void, c_ulong) -> c_long;
    snd_pcm_recover: fn(*mut Pcm, c_int, c_int) -> c_int;
    snd_pcm_delay: fn(*mut Pcm, *mut c_long) -> c_int;
    snd_pcm_drain: fn(*mut Pcm) -> c_int;
    snd_pcm_drop: fn(*mut Pcm) -> c_int;
    snd_pcm_prepare: fn(*mut Pcm) -> c_int;
    snd_strerror: fn(c_int) -> *const c_char;
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(Api::load).as_ref()
}

fn error(api: &Api, code: c_int, context: &str) -> io::Error {
    // SAFETY: snd_strerror returns a static string for any code.
    let message = unsafe { CStr::from_ptr((api.snd_strerror)(code)) }
        .to_string_lossy()
        .into_owned();
    let kind = match -code {
        libc::EBUSY => io::ErrorKind::ResourceBusy,
        libc::ENOENT | libc::ENODEV => io::ErrorKind::NotFound,
        libc::EINVAL => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    };
    let message = if -code == libc::EBUSY {
        format!("{context}: the device is in use by another program ({message})")
    } else {
        format!("{context}: {message}")
    };
    io::Error::new(kind, message)
}

fn take_string(pointer: *mut c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: libasound returns malloc'd NUL-terminated strings for the
    // card name calls; they are copied and then freed here.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { libc::free(pointer.cast()) };
    Some(text)
}

pub struct AlsaBackend;

impl AlsaBackend {
    pub fn load() -> Option<Self> {
        api().map(|_| Self)
    }
}

/// `alsa:USB:0` to `hw:CARD=USB,DEV=0` (or `plughw:` when shared).
fn pcm_name(device_id: &str, exclusive: bool) -> Option<String> {
    let rest = device_id.strip_prefix("alsa:")?;
    let (card, device) = rest.rsplit_once(':')?;
    if card.is_empty()
        || !card
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || device.parse::<u32>().is_err()
    {
        return None;
    }
    let plugin = if exclusive { "hw" } else { "plughw" };
    Some(format!("{plugin}:CARD={card},DEV={device}"))
}

struct CardDevice {
    card: c_int,
    card_id: String,
    card_name: String,
    card_longname: String,
    device: c_int,
    pcm_name: String,
}

fn enumerate(api: &Api) -> Vec<CardDevice> {
    let mut found = Vec::new();
    let mut card: c_int = -1;
    // SAFETY: all pointers passed are valid out-parameters, and every
    // handle opened here is closed before the loop moves on.
    unsafe {
        while (api.snd_card_next)(&mut card) == 0 && card >= 0 {
            let mut name = std::ptr::null_mut();
            let card_name = if (api.snd_card_get_name)(card, &mut name) == 0 {
                take_string(name)
            } else {
                None
            }
            .unwrap_or_else(|| format!("Card {card}"));
            let mut longname = std::ptr::null_mut();
            let card_longname = if (api.snd_card_get_longname)(card, &mut longname) == 0 {
                take_string(longname)
            } else {
                None
            }
            .unwrap_or_default();
            let Ok(ctl_name) = CString::new(format!("hw:{card}")) else {
                continue;
            };
            let mut ctl = std::ptr::null_mut();
            if (api.snd_ctl_open)(&mut ctl, ctl_name.as_ptr(), 0) < 0 {
                continue;
            }
            let mut card_info = std::ptr::null_mut();
            let mut card_id = card.to_string();
            if (api.snd_ctl_card_info_malloc)(&mut card_info) == 0 {
                if (api.snd_ctl_card_info)(ctl, card_info) == 0 {
                    let id = (api.snd_ctl_card_info_get_id)(card_info);
                    if !id.is_null() {
                        card_id = CStr::from_ptr(id).to_string_lossy().into_owned();
                    }
                }
                (api.snd_ctl_card_info_free)(card_info);
            }
            let mut pcm_info = std::ptr::null_mut();
            if (api.snd_pcm_info_malloc)(&mut pcm_info) == 0 {
                let mut device: c_int = -1;
                while (api.snd_ctl_pcm_next_device)(ctl, &mut device) == 0 && device >= 0 {
                    (api.snd_pcm_info_set_device)(pcm_info, device as c_uint);
                    (api.snd_pcm_info_set_subdevice)(pcm_info, 0);
                    (api.snd_pcm_info_set_stream)(pcm_info, STREAM_PLAYBACK);
                    if (api.snd_ctl_pcm_info)(ctl, pcm_info) < 0 {
                        // Capture-only device.
                        continue;
                    }
                    let pcm_name = {
                        let name = (api.snd_pcm_info_get_name)(pcm_info);
                        if name.is_null() {
                            String::new()
                        } else {
                            CStr::from_ptr(name).to_string_lossy().into_owned()
                        }
                    };
                    found.push(CardDevice {
                        card,
                        card_id: card_id.clone(),
                        card_name: card_name.clone(),
                        card_longname: card_longname.clone(),
                        device,
                        pcm_name,
                    });
                }
                (api.snd_pcm_info_free)(pcm_info);
            }
            (api.snd_ctl_close)(ctl);
        }
    }
    found
}

fn probe(api: &Api, name: &str) -> io::Result<DeviceCaps> {
    let c_name = CString::new(name).map_err(|_| io::Error::other("invalid device name"))?;
    let mut pcm = std::ptr::null_mut();
    // SAFETY: the PCM and parameter handles are valid until they are freed
    // at the end of this block, and only read through libasound.
    unsafe {
        let code = (api.snd_pcm_open)(&mut pcm, c_name.as_ptr(), STREAM_PLAYBACK, MODE_NONBLOCK);
        if code < 0 {
            return Err(error(api, code, name));
        }
        let mut params = std::ptr::null_mut();
        if (api.snd_pcm_hw_params_malloc)(&mut params) < 0 {
            (api.snd_pcm_close)(pcm);
            return Err(io::Error::other("out of memory"));
        }
        let code = (api.snd_pcm_hw_params_any)(pcm, params);
        let result = if code < 0 {
            Err(error(api, code, name))
        } else {
            let sample_rates = STANDARD_RATES
                .iter()
                .copied()
                .filter(|rate| (api.snd_pcm_hw_params_test_rate)(pcm, params, *rate, 0) == 0)
                .collect();
            let sample_formats = [
                SampleFormat::S16,
                SampleFormat::S24Packed,
                SampleFormat::S24In32,
                SampleFormat::S32,
            ]
            .into_iter()
            .filter(|format| {
                (api.snd_pcm_hw_params_test_format)(pcm, params, alsa_format(*format)) == 0
            })
            .collect();
            let mut min: c_uint = 0;
            let mut max: c_uint = 0;
            (api.snd_pcm_hw_params_get_channels_min)(params, &mut min);
            (api.snd_pcm_hw_params_get_channels_max)(params, &mut max);
            let dsd_formats: Vec<SampleFormat> = DSD_FORMATS
                .into_iter()
                .filter(|format| {
                    (api.snd_pcm_hw_params_test_format)(pcm, params, alsa_format(*format)) == 0
                })
                .collect();
            let dsd_rates = match dsd_formats.first() {
                Some(preferred) => probe_dsd_rates(api, pcm, params, *preferred),
                None => Vec::new(),
            };
            Ok(DeviceCaps {
                sample_rates,
                sample_formats,
                min_channels: min.clamp(1, 255) as u8,
                max_channels: max.clamp(1, 255) as u8,
                dsd_formats,
                dsd_rates,
            })
        };
        (api.snd_pcm_hw_params_free)(params);
        (api.snd_pcm_close)(pcm);
        result
    }
}

/// DSD rates the device takes in `format`: the rate is tested with the
/// format fixed, because a DSD container's frame rate is its own.
///
/// # Safety
/// `pcm` and `params` must be live handles from [`probe`].
unsafe fn probe_dsd_rates(
    api: &Api,
    pcm: *mut Pcm,
    params: *mut HwParams,
    format: SampleFormat,
) -> Vec<u32> {
    let mut scratch = std::ptr::null_mut();
    // SAFETY: the scratch parameters are freed before returning.
    unsafe {
        if (api.snd_pcm_hw_params_malloc)(&mut scratch) < 0 {
            return Vec::new();
        }
        (api.snd_pcm_hw_params_copy)(scratch, params);
        let rates = if (api.snd_pcm_hw_params_set_format)(pcm, scratch, alsa_format(format)) == 0 {
            DSD_RATES
                .into_iter()
                .filter(|rate| {
                    (api.snd_pcm_hw_params_test_rate)(pcm, scratch, format.dsd_frame_rate(*rate), 0)
                        == 0
                })
                .collect()
        } else {
            Vec::new()
        };
        (api.snd_pcm_hw_params_free)(scratch);
        rates
    }
}

/// The kernel's view of a card: USB descriptors and product name.
fn hardware_info(
    root: &std::path::Path,
    found: &CardDevice,
    probed: bool,
) -> Option<(HardwareInfo, Option<DeviceCaps>)> {
    let (usb_id, stream) = procfs::read_usb(root, &format!("card{}", found.card), found.device)?;
    let caps = procfs::caps_from_usb(&stream);
    Some((
        HardwareInfo {
            bus: "usb".into(),
            usb_id,
            product: stream.product.clone(),
            speed: stream.speed.clone(),
            caps_source: if probed {
                CapsSource::Driver
            } else {
                CapsSource::UsbDescriptors
            },
            altsets: stream.playback,
        },
        caps,
    ))
}

/// `alsa:USB:0` to its card id and device number.
fn card_and_device(device_id: &str) -> Option<(&str, i32)> {
    let rest = device_id.strip_prefix("alsa:")?;
    let (card, device) = rest.rsplit_once(':')?;
    Some((card, device.parse().ok()?))
}

impl Backend for AlsaBackend {
    fn driver(&self) -> Driver {
        Driver::Alsa
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let Some(api) = api() else {
            return Vec::new();
        };
        enumerate(api)
            .into_iter()
            .map(|found| {
                let id = format!("alsa:{}:{}", found.card_id, found.device);
                let hw = format!("hw:CARD={},DEV={}", found.card_id, found.device);
                let (mut caps, probe_error) = match probe(api, &hw) {
                    Ok(caps) => (Some(caps), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                let root = std::path::Path::new(procfs::PROC_ASOUND);
                let hardware = hardware_info(root, &found, caps.is_some()).map(
                    |(hardware, descriptor_caps)| {
                        // A busy DAC cannot be probed, but its descriptors
                        // still say what it takes.
                        if caps.is_none() {
                            caps = descriptor_caps;
                        }
                        hardware
                    },
                );
                let name = match hardware.as_ref().and_then(|hw| hw.product.clone()) {
                    Some(product) => product,
                    None if found.pcm_name.is_empty() || found.pcm_name == found.card_name => {
                        found.card_name.clone()
                    }
                    None => format!("{} {}", found.card_name, found.pcm_name),
                };
                DeviceInfo {
                    id,
                    name,
                    detail: if found.card_longname.is_empty() {
                        hw
                    } else {
                        format!("{hw} · {}", found.card_longname)
                    },
                    driver: Driver::Alsa,
                    exclusive_capable: true,
                    caps,
                    probe_error,
                    is_default: false,
                    hardware,
                }
            })
            .collect()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> io::Result<Box<dyn Sink>> {
        let api = api().ok_or_else(|| io::Error::other("libasound is not available"))?;
        let name = pcm_name(device_id, exclusive)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not an ALSA device id"))?;
        Ok(Box::new(open_pcm(api, &name, format, exclusive)?))
    }

    fn hardware_status(&self, device_id: &str, stream: &StreamFormat) -> Option<HardwareStatus> {
        let (card, device) = card_and_device(device_id)?;
        procfs::read_status(
            std::path::Path::new(procfs::PROC_ASOUND),
            card,
            device,
            stream,
        )
    }
}

/// Opens an ALSA PCM by name for `format`. With `exclusive`, ALSA's
/// soft_resample is off, so a rate the PCM cannot take is an error
/// instead of a silent conversion.
fn open_pcm(
    api: &'static Api,
    name: &str,
    format: StreamFormat,
    exclusive: bool,
) -> io::Result<AlsaSink> {
    let c_name = CString::new(name).map_err(|_| io::Error::other("invalid name"))?;
    let mut pcm = std::ptr::null_mut();
    // SAFETY: the handle is owned by the returned sink, which closes it.
    unsafe {
        let code = (api.snd_pcm_open)(&mut pcm, c_name.as_ptr(), STREAM_PLAYBACK, 0);
        if code < 0 {
            return Err(error(api, code, name));
        }
        let code = (api.snd_pcm_set_params)(
            pcm,
            alsa_format(format.sample_format),
            ACCESS_RW_INTERLEAVED,
            c_uint::from(format.channels),
            format.sample_rate,
            c_int::from(!exclusive),
            LATENCY_US,
        );
        if code < 0 {
            (api.snd_pcm_close)(pcm);
            return Err(error(api, code, name));
        }
    }
    Ok(AlsaSink { api, pcm, format })
}

struct AlsaSink {
    api: &'static Api,
    pcm: *mut Pcm,
    format: StreamFormat,
}

// SAFETY: the PCM handle is only used by the thread that owns the sink;
// libasound allows a handle to move between threads.
unsafe impl Send for AlsaSink {}

impl Sink for AlsaSink {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let frame = self.format.frame_bytes();
        let mut offset = 0;
        while offset + frame <= data.len() {
            let frames = ((data.len() - offset) / frame) as c_ulong;
            // SAFETY: the buffer holds at least `frames` whole frames.
            let written = unsafe {
                (self.api.snd_pcm_writei)(self.pcm, data[offset..].as_ptr().cast(), frames)
            };
            if written < 0 {
                let code = written as c_int;
                // Underrun or suspend: recover and retry.
                // SAFETY: the handle is open.
                let recovered = unsafe { (self.api.snd_pcm_recover)(self.pcm, code, 1) };
                if recovered < 0 {
                    return Err(error(self.api, code, "ALSA write"));
                }
                continue;
            }
            offset += written as usize * frame;
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        let mut delay: c_long = 0;
        // SAFETY: the handle is open and `delay` is a valid out-parameter.
        if unsafe { (self.api.snd_pcm_delay)(self.pcm, &mut delay) } < 0 {
            return 0;
        }
        delay.max(0) as u64
    }

    fn flush(&mut self) -> io::Result<()> {
        // SAFETY: the handle is open.
        unsafe {
            (self.api.snd_pcm_drop)(self.pcm);
            (self.api.snd_pcm_prepare)(self.pcm);
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        // SAFETY: the handle is open.
        unsafe {
            (self.api.snd_pcm_drain)(self.pcm);
            (self.api.snd_pcm_prepare)(self.pcm);
        }
        Ok(())
    }
}

impl Drop for AlsaSink {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed once.
        unsafe {
            (self.api.snd_pcm_drop)(self.pcm);
            (self.api.snd_pcm_close)(self.pcm);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn device_ids_map_to_hw_and_plughw_names() {
        assert_eq!(
            pcm_name("alsa:USB:0", true).as_deref(),
            Some("hw:CARD=USB,DEV=0")
        );
        assert_eq!(
            pcm_name("alsa:PCH:3", false).as_deref(),
            Some("plughw:CARD=PCH,DEV=3")
        );
        assert_eq!(pcm_name("alsa:USB", true), None);
        assert_eq!(pcm_name("alsa:U,S:0", true), None);
        assert_eq!(pcm_name("wasapi:x:0", true), None);
    }

    #[test]
    fn enumeration_works_without_sound_cards() {
        // Machines without libasound get no backend; with it, listing must
        // not fail even when there are no cards (CI, containers).
        if let Some(backend) = AlsaBackend::load() {
            for device in backend.devices() {
                assert!(device.id.starts_with("alsa:"));
            }
        }
    }

    #[test]
    fn the_null_pcm_plays_through_the_sink() {
        // ALSA's software "null" PCM accepts any format without hardware,
        // which exercises open, set_params, writei and drain for real.
        let Some(api) = api() else {
            return;
        };
        let mut pcm = std::ptr::null_mut();
        // SAFETY: as in `open`.
        let opened = unsafe { (api.snd_pcm_open)(&mut pcm, c"null".as_ptr(), STREAM_PLAYBACK, 0) };
        if opened < 0 {
            return;
        }
        let format = StreamFormat {
            sample_rate: 44_100,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        // SAFETY: as in `open`.
        let code = unsafe {
            (api.snd_pcm_set_params)(
                pcm,
                FORMAT_S16_LE,
                ACCESS_RW_INTERLEAVED,
                2,
                44_100,
                0,
                LATENCY_US,
            )
        };
        assert_eq!(code, 0);
        let mut sink = AlsaSink { api, pcm, format };
        sink.write(&vec![0; 4_410 * 4]).unwrap();
        sink.flush().unwrap();
        sink.drain().unwrap();
    }

    /// Plays `pcm` through the real libasound `file` plugin (a software
    /// PCM that records exactly what the driver layer receives) and
    /// returns the bytes it recorded.
    fn record_through_alsa(
        format: StreamFormat,
        input: crate::player::InputFormat,
        pcm: impl FnMut(u64) -> io::Result<crate::player::PcmSource> + Send + 'static,
    ) -> Option<Vec<u8>> {
        let api = api()?;
        let path = std::env::temp_dir().join(format!(
            "parson-alsa-{}-{}.raw",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        let name = format!("file:FILE={},FORMAT=raw", path.display());
        // Probe first: a libasound without its configuration has no `file`.
        drop(open_pcm(api, &name, format, true).ok()?);
        let _ = std::fs::remove_file(&path);
        let player = crate::player::Player::start(
            Box::new(move || {
                open_pcm(api, &name, format, true).map(|sink| Box::new(sink) as Box<dyn Sink>)
            }),
            Box::new(pcm),
            format,
            input,
            0,
        )
        .expect("the file PCM opens in exclusive mode");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while player.status().state != crate::player::PlayerState::Ended {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                player.status()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        player.stop();
        let recorded = std::fs::read(&path).expect("the file plugin wrote the stream");
        let _ = std::fs::remove_file(&path);
        Some(recorded)
    }

    #[test]
    fn untouched_pcm_reaches_libasound_bit_for_bit() {
        let samples: Vec<u8> = (0..44_100 / 5)
            .flat_map(|n: i32| {
                let value = ((n * 7_919) % 65_536 - 32_768) as i16;
                [value.to_le_bytes(), (!value).to_le_bytes()].concat()
            })
            .collect();
        let format = StreamFormat {
            sample_rate: 44_100,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        let source = samples.clone();
        let Some(recorded) =
            record_through_alsa(format, crate::player::InputFormat::S16, move |_| {
                Ok(Box::new(io::Cursor::new(source.clone())) as crate::player::PcmSource)
            })
        else {
            return;
        };
        assert_eq!(recorded.len(), samples.len());
        assert!(recorded == samples, "ALSA received different samples");
    }

    #[test]
    fn equalised_audio_reaches_libasound_exactly_as_the_dsp_computed_it() {
        use crate::dsp::{DspChain, DspPcm, DspStats, EqBand, EqFilterType, ProcessingPlan};
        use std::sync::Arc;
        let plan = ProcessingPlan {
            leveling_db: None,
            headroom_db: Some(-6.0),
            bands: vec![EqBand {
                id: "band-1".into(),
                enabled: true,
                filter_type: EqFilterType::Lowshelf,
                frequency: 80.0,
                gain_db: 5.0,
                q: 0.71,
            }],
            crossfeed: Some(crate::dsp::CrossfeedStrength::Medium),
            crossfeed_skipped: false,
            channels: 2,
            input_rate: 44_100,
            output_rate: 96_000,
            rate_requested: true,
            requested_rate_unavailable: None,
            resampler: Some(crate::dsp::ResamplerKind::Parson),
            output_bits: 24,
        };
        let floats: Vec<u8> = (0..44_100 / 5)
            .flat_map(|n| {
                let t = f64::from(n) / 44_100.0;
                let left = 0.4 * (2.0 * std::f64::consts::PI * 60.0 * t).sin();
                let right = 0.3 * (2.0 * std::f64::consts::PI * 3_000.0 * t).sin();
                [left.to_le_bytes(), right.to_le_bytes()].concat()
            })
            .collect();
        // What the chain computes, packed for a 24-bit packed device.
        let mut expected = Vec::new();
        DspPcm::new(
            Box::new(io::Cursor::new(floats.clone())),
            DspChain::new(
                &plan,
                44_100,
                crate::player::InputFormat::S32,
                Arc::new(DspStats::default()),
            ),
        )
        .read_to_end(&mut expected)
        .unwrap();
        let mut packed = Vec::new();
        crate::pack_samples(&expected, SampleFormat::S24Packed, &mut packed);
        let format = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        let stats = Arc::new(DspStats::default());
        let chain_stats = Arc::clone(&stats);
        let Some(recorded) =
            record_through_alsa(format, crate::player::InputFormat::S32, move |_| {
                Ok(Box::new(DspPcm::new(
                    Box::new(io::Cursor::new(floats.clone())),
                    DspChain::new(
                        &plan,
                        44_100,
                        crate::player::InputFormat::S32,
                        Arc::clone(&chain_stats),
                    ),
                )) as crate::player::PcmSource)
            })
        else {
            return;
        };
        assert_eq!(recorded.len(), packed.len());
        assert!(recorded == packed, "ALSA received different samples");
        let status = stats.snapshot();
        assert!(status.realtime_factor.is_some_and(|factor| factor > 1.0));
        assert!(!status.clipping);
    }

    /// A DSF file played the way the server plays it: read, packed and
    /// written by the real player thread into libasound.
    fn dsd_through_alsa(
        packing: crate::dsd::DsdPacking,
        frame_rate: u32,
        bytes: &[u8],
    ) -> Option<Vec<u8>> {
        use crate::dsd::{DsdPcm, DsdReader, read_info, write_dsf};
        let file = write_dsf(2_822_400, 2, bytes);
        let container = match packing {
            crate::dsd::DsdPacking::Native(format) | crate::dsd::DsdPacking::Dop(format) => format,
        };
        let format = StreamFormat {
            sample_rate: frame_rate,
            sample_format: container,
            channels: 2,
        };
        record_through_alsa(format, crate::player::InputFormat::Raw, move |_| {
            let mut cursor = io::Cursor::new(file.clone());
            let info = read_info(&mut cursor)?;
            Ok(
                Box::new(DsdPcm::new(DsdReader::new(cursor, info, 0), 2, packing))
                    as crate::player::PcmSource,
            )
        })
    }

    fn dsd_pattern() -> Vec<u8> {
        // 0.1 s of DSD64 stereo, as whole 32-bit words.
        (0..35_280_usize)
            .flat_map(|n| [(n * 37 % 251) as u8, (n * 91 % 241 + 3) as u8])
            .collect()
    }

    #[test]
    fn dop_reaches_libasound_with_every_dsd_bit_and_marker() {
        let bytes = dsd_pattern();
        let Some(recorded) = dsd_through_alsa(
            crate::dsd::DsdPacking::Dop(SampleFormat::S24Packed),
            176_400,
            &bytes,
        ) else {
            return;
        };
        let decoded = crate::dsd::unpack_dop(&recorded, 2, SampleFormat::S24Packed)
            .expect("the markers alternate, so a DAC sees DoP");
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn native_dsd_reaches_libasound_unchanged() {
        let bytes = dsd_pattern();
        let Some(recorded) = dsd_through_alsa(
            crate::dsd::DsdPacking::Native(SampleFormat::DsdU32Be),
            88_200,
            &bytes,
        ) else {
            return;
        };
        // Undo DSD_U32_BE's per-channel words to compare in time order.
        let mut restored = Vec::with_capacity(recorded.len());
        for frame in recorded.as_chunks::<8>().0 {
            for time in 0..4 {
                restored.push(frame[time]);
                restored.push(frame[4 + time]);
            }
        }
        assert_eq!(restored, bytes);
    }
}
