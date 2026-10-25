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

use crate::{
    Backend, DeviceCaps, DeviceInfo, Driver, STANDARD_RATES, SampleFormat, Sink, StreamFormat,
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
/// Requested device buffer, in microseconds.
const LATENCY_US: c_uint = 200_000;

fn alsa_format(format: SampleFormat) -> c_int {
    match format {
        SampleFormat::S16 => FORMAT_S16_LE,
        SampleFormat::S24Packed => FORMAT_S24_3LE,
        SampleFormat::S24In32 => FORMAT_S24_LE,
        SampleFormat::S32 => FORMAT_S32_LE,
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
            Ok(DeviceCaps {
                sample_rates,
                sample_formats,
                min_channels: min.clamp(1, 255) as u8,
                max_channels: max.clamp(1, 255) as u8,
            })
        };
        (api.snd_pcm_hw_params_free)(params);
        (api.snd_pcm_close)(pcm);
        result
    }
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
                let (caps, probe_error) = match probe(api, &hw) {
                    Ok(caps) => (Some(caps), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                let name = if found.pcm_name.is_empty() || found.pcm_name == found.card_name {
                    found.card_name.clone()
                } else {
                    format!("{} {}", found.card_name, found.pcm_name)
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
        let c_name = CString::new(name.clone()).map_err(|_| io::Error::other("invalid name"))?;
        let mut pcm = std::ptr::null_mut();
        // SAFETY: the handle is owned by the returned sink, which closes it.
        unsafe {
            let code = (api.snd_pcm_open)(&mut pcm, c_name.as_ptr(), STREAM_PLAYBACK, 0);
            if code < 0 {
                return Err(error(api, code, &name));
            }
            // soft_resample is off in exclusive mode: set_params fails rather
            // than play at a rate other than the one asked for.
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
                return Err(error(api, code, &name));
            }
        }
        Ok(Box::new(AlsaSink { api, pcm, format }))
    }
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
}
