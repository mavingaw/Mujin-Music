//! ALSA `hw:` output, with libasound loaded at run time.
//!
//! A `hw:` PCM talks to the driver directly: there is no dmix mixer, no
//! rate converter and no format plug in front of it, so a format it accepts
//! reaches the DAC bit for bit and only one program can use it at a time.
//! `plughw:` is the same device with ALSA's conversion plugins enabled,
//! used when exclusive mode is off.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::pcm_chain::{PcmChain, parse_pcm_dump};
use crate::procfs::{self, CapsSource, HardwareInfo, HardwareReporter, HardwareStatus};
use crate::{
    ALSA_CONFIG_PCM_PREFIX, Backend, DSD_FORMATS, DSD_RATES, DeviceCaps, DeviceInfo, Driver,
    STANDARD_RATES, SampleFormat, Sink, StreamFormat,
};

type Ctl = c_void;
type Pcm = c_void;
type PcmInfo = c_void;
type CardInfo = c_void;
type HwParams = c_void;
type Output = c_void;

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
    snd_card_get_index: fn(*const c_char) -> c_int;
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
    snd_device_name_hint: fn(c_int, *const c_char, *mut *mut *mut c_void) -> c_int;
    snd_device_name_get_hint: fn(*const c_void, *const c_char) -> *mut c_char;
    snd_device_name_free_hint: fn(*mut *mut c_void) -> c_int;
    snd_output_buffer_open: fn(*mut *mut Output) -> c_int;
    snd_output_buffer_string: fn(*mut Output, *mut *mut c_char) -> usize;
    snd_output_close: fn(*mut Output) -> c_int;
    snd_pcm_dump: fn(*mut Pcm, *mut Output) -> c_int;
}

/// The card Parson's kernel-ABI emulator answers for, when it is loaded
/// into this process (`kernel-abi/snd_usb_emulator.c`, via `LD_PRELOAD`):
/// that card is no kernel driver and no DAC, and Parson says so.
pub(crate) fn emulated_card() -> Option<c_int> {
    static CARD: OnceLock<Option<c_int>> = OnceLock::new();
    *CARD.get_or_init(|| {
        // SAFETY: dlsym only looks the symbol up; the emulator defines it
        // as a `const int`.
        unsafe {
            let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"parson_snd_usb_emulator_card".as_ptr());
            (!symbol.is_null()).then(|| *symbol.cast::<c_int>())
        }
    })
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

/// What libasound built when a device was opened: the PCM name and the
/// plugin chain, with the setup the bottom of the chain negotiated.
#[derive(Clone, Debug)]
struct OpenReport {
    pcm: String,
    chain: PcmChain,
    generation: u64,
}

type OpenReports = Arc<Mutex<HashMap<String, OpenReport>>>;

pub struct AlsaBackend {
    open: OpenReports,
}

impl AlsaBackend {
    pub fn load() -> Option<Self> {
        api().map(|_| Self {
            open: Arc::default(),
        })
    }
}

/// A PCM name from the ALSA configuration that Parson can open as is:
/// letters, digits, `_`, `-` and `.`, the way `pcm.<name>` is written.
fn valid_config_pcm(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Configuration PCMs worth listing beside the sound cards: named ones a
/// user or a package defined (a network bridge, a loopback, a capture, a
/// sound server). Per-card aliases (`front:CARD=…`, `hw:…`) are covered by
/// the card list, and `null`/`default`/`sysdefault` are not outputs.
fn listable_config_pcm(name: &str, ioid: Option<&str>) -> bool {
    valid_config_pcm(name)
        && !matches!(name, "null" | "default" | "sysdefault")
        && ioid.is_none_or(|ioid| ioid == "Output")
}

/// `alsa:USB:0` to `hw:CARD=USB,DEV=0` (or `plughw:` when shared), and
/// `alsa-pcm:studio` to `studio` (or `plug:studio` when shared, which lets
/// ALSA convert).
fn pcm_name(device_id: &str, exclusive: bool) -> Option<String> {
    if let Some(name) = device_id.strip_prefix(ALSA_CONFIG_PCM_PREFIX) {
        if !valid_config_pcm(name) {
            return None;
        }
        return Some(if exclusive {
            name.to_string()
        } else {
            format!("plug:{name}")
        });
    }
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

/// PCMs from the ALSA configuration, as `aplay -L` lists them: name and
/// the first line of the description.
fn enumerate_config_pcms(api: &Api) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let mut hints: *mut *mut c_void = std::ptr::null_mut();
    // SAFETY: libasound fills `hints` with a NULL-terminated array that is
    // freed below; each hint string it returns is malloc'd and taken.
    unsafe {
        if (api.snd_device_name_hint)(-1, c"pcm".as_ptr(), &mut hints) < 0 || hints.is_null() {
            return found;
        }
        let mut cursor = hints;
        while !(*cursor).is_null() {
            let hint = *cursor;
            let name = take_string((api.snd_device_name_get_hint)(hint, c"NAME".as_ptr()));
            let description = take_string((api.snd_device_name_get_hint)(hint, c"DESC".as_ptr()));
            let ioid = take_string((api.snd_device_name_get_hint)(hint, c"IOID".as_ptr()));
            if let Some(name) = name
                && listable_config_pcm(&name, ioid.as_deref())
            {
                let description = description
                    .and_then(|text| text.lines().next().map(str::trim).map(str::to_string))
                    .filter(|text| !text.is_empty());
                found.push((name, description));
            }
            cursor = cursor.add(1);
        }
        (api.snd_device_name_free_hint)(hints);
    }
    found
}

/// The chain libasound built for an open, configured PCM.
///
/// # Safety
/// `pcm` must be a live handle whose parameters are set.
unsafe fn dump_chain(api: &Api, pcm: *mut Pcm) -> Option<PcmChain> {
    let mut output = std::ptr::null_mut();
    // SAFETY: the output buffer is closed before returning; the string it
    // hands out stays valid until then and is copied first.
    unsafe {
        if (api.snd_output_buffer_open)(&mut output) < 0 {
            return None;
        }
        let text = if (api.snd_pcm_dump)(pcm, output) == 0 {
            let mut buffer: *mut c_char = std::ptr::null_mut();
            let length = (api.snd_output_buffer_string)(output, &mut buffer);
            (!buffer.is_null()).then(|| {
                String::from_utf8_lossy(std::slice::from_raw_parts(buffer.cast::<u8>(), length))
                    .into_owned()
            })
        } else {
            None
        };
        (api.snd_output_close)(output);
        text.map(|text| parse_pcm_dump(&text))
    }
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
            kernel_emulator: emulated_card() == Some(found.card),
        },
        caps,
    ))
}

fn lock(reports: &OpenReports) -> std::sync::MutexGuard<'_, HashMap<String, OpenReport>> {
    reports
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// libasound's account of an open PCM: the setup the bottom of its chain
/// negotiated, which matches the stream only when no plugin on the way
/// could change a sample.
fn library_status(report: &OpenReport, stream: &StreamFormat) -> Option<HardwareStatus> {
    let params = report.chain.bottom.clone()?;
    Some(HardwareStatus {
        source: format!("snd_pcm_dump({})", report.pcm),
        reporter: HardwareReporter::Library,
        matches_stream: report.chain.bit_exact && procfs::params_match(&params, stream),
        chain: Some(report.chain.clone()),
        params,
        momentary_hz: None,
        kernel_emulator: false,
    })
}

/// The index of the card with id `card` (e.g. `BenchDAC` to 1).
fn card_index(card: &str) -> Option<c_int> {
    let api = api()?;
    let name = CString::new(card).ok()?;
    // SAFETY: `name` is NUL-terminated and outlives the call.
    let index = unsafe { (api.snd_card_get_index)(name.as_ptr()) };
    (index >= 0).then_some(index)
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
        let cards = enumerate(api).into_iter().map(|found| {
            let id = format!("alsa:{}:{}", found.card_id, found.device);
            let hw = format!("hw:CARD={},DEV={}", found.card_id, found.device);
            let (mut caps, probe_error) = match probe(api, &hw) {
                Ok(caps) => (Some(caps), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let root = std::path::Path::new(procfs::PROC_ASOUND);
            let hardware =
                hardware_info(root, &found, caps.is_some()).map(|(hardware, descriptor_caps)| {
                    // A busy DAC cannot be probed, but its descriptors
                    // still say what it takes.
                    if caps.is_none() {
                        caps = descriptor_caps;
                    }
                    hardware
                });
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
                address: Some(format!("hw:{},{}", found.card, found.device)),
                network: None,
            }
        });
        let configured = enumerate_config_pcms(api)
            .into_iter()
            .map(|(pcm, description)| {
                let (caps, probe_error) = match probe(api, &pcm) {
                    Ok(caps) => (Some(caps), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                DeviceInfo {
                    id: format!("{ALSA_CONFIG_PCM_PREFIX}{pcm}"),
                    name: description.clone().unwrap_or_else(|| pcm.clone()),
                    detail: format!("ALSA PCM \"{pcm}\" from the ALSA configuration"),
                    driver: Driver::Alsa,
                    exclusive_capable: true,
                    caps,
                    probe_error,
                    is_default: false,
                    hardware: None,
                    address: Some(pcm),
                    network: None,
                }
            });
        cards.chain(configured).collect()
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
        let mut sink = open_pcm(api, &name, format, exclusive)?;
        // SAFETY: the sink's handle is open and its parameters are set.
        if let Some(chain) = unsafe { dump_chain(api, sink.pcm) } {
            static GENERATION: AtomicU64 = AtomicU64::new(0);
            let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
            lock(&self.open).insert(
                device_id.to_string(),
                OpenReport {
                    pcm: name,
                    chain,
                    generation,
                },
            );
            sink.report = Some((Arc::clone(&self.open), device_id.to_string(), generation));
        }
        Ok(Box::new(sink))
    }

    fn hardware_status(&self, device_id: &str, stream: &StreamFormat) -> Option<HardwareStatus> {
        // A sound card: the kernel's own hw_params first.
        if let Some((card, device)) = card_and_device(device_id)
            && let Some(mut status) = procfs::read_status(
                std::path::Path::new(procfs::PROC_ASOUND),
                card,
                device,
                stream,
            )
        {
            status.kernel_emulator = emulated_card().is_some_and(|emulated| {
                card == emulated.to_string() || card_index(card) == Some(emulated)
            });
            return Some(status);
        }
        let report = lock(&self.open).get(device_id).cloned()?;
        library_status(&report, stream)
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
    Ok(AlsaSink {
        api,
        pcm,
        format,
        report: None,
    })
}

struct AlsaSink {
    api: &'static Api,
    pcm: *mut Pcm,
    format: StreamFormat,
    /// The open report this sink registered, removed when it closes
    /// (unless a newer open of the device replaced it).
    report: Option<(OpenReports, String, u64)>,
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
        if let Some((reports, device_id, generation)) = self.report.take() {
            let mut reports = lock(&reports);
            if reports
                .get(&device_id)
                .is_some_and(|report| report.generation == generation)
            {
                reports.remove(&device_id);
            }
        }
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
        assert_eq!(
            pcm_name("alsa-pcm:studio_dac", true).as_deref(),
            Some("studio_dac")
        );
        assert_eq!(
            pcm_name("alsa-pcm:studio_dac", false).as_deref(),
            Some("plug:studio_dac")
        );
        assert_eq!(pcm_name("alsa-pcm:file:FILE=/etc/passwd", true), None);
        assert_eq!(pcm_name("alsa-pcm:", true), None);
        assert_eq!(card_and_device("alsa-pcm:studio"), None);
    }

    #[test]
    fn only_named_output_pcms_from_the_configuration_are_listed() {
        assert!(listable_config_pcm("parson_capture", None));
        assert!(listable_config_pcm("pipewire", Some("Output")));
        assert!(!listable_config_pcm("mic", Some("Input")));
        assert!(!listable_config_pcm("null", None));
        assert!(!listable_config_pcm("default", None));
        assert!(!listable_config_pcm("front:CARD=PCH,DEV=0", None));
        assert!(!listable_config_pcm("hw:CARD=E30,DEV=0", None));
    }

    #[test]
    fn libasound_reports_the_chain_it_opened() {
        // The real libasound builds a file -> null chain; Parson reads it
        // back with snd_pcm_dump and checks the bottom's setup.
        let Some(api) = api() else {
            return;
        };
        let path = std::env::temp_dir().join(format!("parson-chain-{}.raw", std::process::id()));
        let format = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        let name = format!("file:FILE={},FORMAT=raw", path.display());
        let Ok(sink) = open_pcm(api, &name, format, true) else {
            return;
        };
        // SAFETY: the sink is open with its parameters set.
        let chain = unsafe { dump_chain(api, sink.pcm) }.expect("libasound dumps the PCM");
        drop(sink);
        let _ = std::fs::remove_file(&path);
        assert_eq!(chain.plugins, ["File PCM", "Null PCM"]);
        assert!(chain.bit_exact);
        let report = OpenReport {
            pcm: name.clone(),
            chain,
            generation: 0,
        };
        let status = library_status(&report, &format).unwrap();
        assert_eq!(status.reporter, HardwareReporter::Library);
        assert_eq!(status.params.format, "S24_3LE");
        assert_eq!(status.params.rate, 96_000);
        assert!(status.matches_stream);
        assert_eq!(status.source, format!("snd_pcm_dump({name})"));
        // Opened as 16-bit, the same chain does not match a 24-bit stream.
        let other = StreamFormat {
            sample_format: SampleFormat::S16,
            ..format
        };
        assert!(!library_status(&report, &other).unwrap().matches_stream);
    }

    #[test]
    fn a_converting_chain_never_matches() {
        let report = OpenReport {
            pcm: "studio".into(),
            chain: crate::pcm_chain::parse_pcm_dump(
                "Plug PCM: Linear conversion PCM (S32_LE)\nIts setup is:\n  format : S16_LE\n  channels : 2\n  rate : 44100\nSlave: Null PCM\nIts setup is:\n  format : S16_LE\n  channels : 2\n  rate : 44100\n",
            ),
            generation: 0,
        };
        let stream = StreamFormat {
            sample_rate: 44_100,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        let status = library_status(&report, &stream).unwrap();
        assert!(!status.matches_stream);
    }

    #[test]
    fn enumeration_works_without_sound_cards() {
        // Machines without libasound get no backend; with it, listing must
        // not fail even when there are no cards (CI, containers).
        if let Some(backend) = AlsaBackend::load() {
            for device in backend.devices() {
                assert!(device.id.starts_with("alsa:") || device.is_alsa_config_pcm());
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
        let mut sink = AlsaSink {
            api,
            pcm,
            format,
            report: None,
        };
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
            convolution: None,
            convolution_skipped: None,
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

    /// Set in the child process that runs with the kernel-ABI emulator.
    const KERNEL_ABI_CHILD: &str = "PARSON_KERNEL_ABI_CHILD";
    const KERNEL_ABI_CAPTURE: &str = "PARSON_SND_EMULATOR_CAPTURE";

    /// Builds `kernel-abi/snd_usb_emulator.c` with the system C compiler.
    fn build_kernel_abi_emulator(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        let library = dir.join("libparson_snd_usb_emulator.so");
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/kernel-abi/snd_usb_emulator.c");
        let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".into());
        let built = std::process::Command::new(compiler)
            .args(["-shared", "-fPIC", "-O2", "-Wall", "-Werror", "-o"])
            .arg(&library)
            .arg(source)
            .args(["-ldl", "-lpthread"])
            .status()
            .is_ok_and(|status| status.success());
        built.then_some(library)
    }

    /// Plays `samples` (already in the device's container) through the
    /// real Player and the backend's own `open`, checking the driver's
    /// report while it plays; returns what the emulated DAC consumed.
    fn play_through_hw(
        backend: &AlsaBackend,
        device_id: &str,
        format: StreamFormat,
        input: crate::player::InputFormat,
        decoded: Vec<u8>,
        capture: &std::path::Path,
    ) -> Vec<u8> {
        let _ = std::fs::remove_file(capture);
        let opener_backend = AlsaBackend {
            open: Arc::clone(&backend.open),
        };
        let id = device_id.to_string();
        let player = crate::player::Player::start(
            Box::new(move || opener_backend.open(&id, format, true)),
            Box::new(move |_| {
                Ok(Box::new(io::Cursor::new(decoded.clone())) as crate::player::PcmSource)
            }),
            format,
            input,
            0,
        )
        .expect("the hw PCM opens in exclusive mode");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut confirmed = None;
        let mut busy = None;
        while player.status().state != crate::player::PlayerState::Ended {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                player.status()
            );
            if confirmed.is_none()
                && player.status().state == crate::player::PlayerState::Playing
                && let Some(status) = backend.hardware_status(device_id, &format)
                && status.momentary_hz.is_some()
            {
                confirmed = Some(status);
                // Exclusive: a second open is refused by the driver.
                busy = Some(backend.open(device_id, format, true).is_err());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        player.stop();
        let status = confirmed.expect("the driver reported the stream while it played");
        assert_eq!(status.reporter, HardwareReporter::Kernel);
        assert!(
            status.kernel_emulator,
            "the emulator is named as the reporter"
        );
        assert!(status.matches_stream, "{status:?}");
        assert_eq!(
            status.params.format,
            procfs::alsa_format_name(format.sample_format)
        );
        assert_eq!(status.params.rate, format.sample_rate);
        assert_eq!(status.momentary_hz, Some(format.sample_rate));
        assert!(
            status.source.ends_with("/pcm0p/sub0/hw_params"),
            "{}",
            status.source
        );
        assert_eq!(busy, Some(true), "a hw device takes one stream at a time");
        std::fs::read(capture).expect("the emulated DAC consumed the stream")
    }

    fn kernel_abi_child() {
        let capture = std::path::PathBuf::from(
            std::env::var_os(KERNEL_ABI_CAPTURE).expect("the parent names the capture file"),
        );
        assert_eq!(emulated_card(), Some(1), "the emulator is loaded");
        let backend = AlsaBackend::load().expect("libasound loads");
        let devices = backend.devices();
        let dac = devices
            .iter()
            .find(|device| {
                device
                    .hardware
                    .as_ref()
                    .is_some_and(|hw| hw.kernel_emulator)
            })
            .unwrap_or_else(|| panic!("the emulated USB card is listed: {devices:?}"));
        assert_eq!(dac.id, "alsa:BenchDAC:0");
        assert_eq!(dac.name, "Parson Bench DAC (kernel ABI emulator)");
        assert_eq!(dac.address.as_deref(), Some("hw:1,0"));
        assert_eq!(dac.mode_label(true), "ALSA hw:1,0 exclusive");
        let hardware = dac.hardware.as_ref().unwrap();
        assert_eq!(hardware.usb_id.as_deref(), Some("1209:0001"));
        assert_eq!(hardware.speed.as_deref(), Some("high speed"));
        assert_eq!(hardware.caps_source, CapsSource::Driver);
        let caps = dac.caps.as_ref().expect("the driver was probed");
        assert_eq!(
            caps.sample_formats,
            [
                SampleFormat::S16,
                SampleFormat::S24Packed,
                SampleFormat::S32
            ]
        );
        assert!(caps.sample_rates.contains(&44_100) && caps.sample_rates.contains(&384_000));
        assert_eq!((caps.min_channels, caps.max_channels), (2, 2));

        // A 44.1kHz/16-bit track goes out as S16_LE, untouched.
        let cd: Vec<u8> = (0..44_100 / 4)
            .flat_map(|n: i32| {
                let value = ((n * 7_919) % 65_536 - 32_768) as i16;
                [value.to_le_bytes(), (!value).to_le_bytes()].concat()
            })
            .collect();
        let cd_format = StreamFormat {
            sample_rate: 44_100,
            sample_format: SampleFormat::S16,
            channels: 2,
        };
        let consumed = play_through_hw(
            &backend,
            &dac.id,
            cd_format,
            crate::player::InputFormat::S16,
            cd.clone(),
            &capture,
        );
        assert_eq!(consumed.len(), cd.len());
        assert!(consumed == cd, "the DAC received different 16-bit samples");

        // A 96kHz/24-bit track: 24 significant bits, packed as S24_3LE.
        let samples: Vec<i32> = (0..96_000 / 4 * 2)
            .map(|n: i32| ((n.wrapping_mul(2_654_435_761_u32 as i32)) >> 8) << 8)
            .collect();
        let decoded: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let expected: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes()[1..].to_vec())
            .collect();
        let hires = StreamFormat {
            sample_rate: 96_000,
            sample_format: SampleFormat::S24Packed,
            channels: 2,
        };
        let consumed = play_through_hw(
            &backend,
            &dac.id,
            hires,
            crate::player::InputFormat::S32,
            decoded,
            &capture,
        );
        assert_eq!(consumed.len(), expected.len());
        assert!(
            consumed == expected,
            "the DAC received different 24-bit samples"
        );

        // Exclusive mode never converts: a rate the 24-bit altset lacks fails.
        let unsupported = StreamFormat {
            sample_rate: 384_000,
            ..hires
        };
        assert!(backend.open(&dac.id, unsupported, true).is_err());
        // Shared mode reaches it through ALSA's plug, which converts, and
        // the driver's report shows it: the hardware runs S32_LE.
        let shared = backend
            .open(&dac.id, unsupported, false)
            .expect("plughw converts what the hardware lacks");
        let report = backend
            .hardware_status(&dac.id, &unsupported)
            .expect("the driver reports the shared stream");
        assert_eq!(report.params.format, "S32_LE");
        assert!(
            !report.matches_stream,
            "a conversion below Parson is caught"
        );
        drop(shared);
    }

    #[test]
    fn the_hw_path_plays_bit_perfect_through_the_kernel_abi() {
        // This container class has no sound cards, so the kernel side of a
        // USB DAC is played by kernel-abi/snd_usb_emulator.c, preloaded
        // into a child copy of this test: the real libasound hw plugin,
        // procfs readers, Player and Backend run on top of it unchanged.
        if std::env::var_os(KERNEL_ABI_CHILD).is_some() {
            kernel_abi_child();
            return;
        }
        if api().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("parson-kernel-abi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let Some(library) = build_kernel_abi_emulator(&dir) else {
            eprintln!("no C compiler or kernel headers; the kernel-ABI path was not exercised");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "alsa::tests::the_hw_path_plays_bit_perfect_through_the_kernel_abi",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(KERNEL_ABI_CHILD, "1")
            .env(KERNEL_ABI_CAPTURE, dir.join("dac.raw"))
            .env("LD_PRELOAD", &library)
            .output()
            .expect("the child test runs");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}
