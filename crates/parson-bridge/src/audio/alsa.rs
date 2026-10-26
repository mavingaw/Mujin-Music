//! ALSA output for Linux, loaded at run time (`libasound.so.2`) so the
//! server builds and runs on machines without ALSA.
//!
//! Each playback PCM of each sound card is a device. Exclusive mode opens
//! `hw:CARD,DEV` directly: no dmix, no plug conversion, the device runs at
//! the file's own rate and depth (bit-perfect), and other apps get "busy"
//! until Parson lets go at the end of the queue. Shared mode opens
//! `plughw:` instead, which converts what the hardware cannot take.
//! Volume uses the card's own mixer in dB when it has one.

#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::Mutex;

use super::{DeviceBackend, DeviceInfo, HardwareVolume, OutputStream, SampleFormat, StreamFormat};

type snd_pcm_t = c_void;
type snd_pcm_hw_params_t = c_void;
type snd_ctl_t = c_void;
type snd_pcm_info_t = c_void;
type snd_mixer_t = c_void;
type snd_mixer_elem_t = c_void;

const SND_PCM_STREAM_PLAYBACK: c_int = 0;
const SND_PCM_NONBLOCK: c_int = 1;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_FORMAT_S32_LE: c_int = 10;
const SND_PCM_FORMAT_S24_3LE: c_int = 32;
const EBUSY: c_int = 16;
const EPIPE: c_int = 32;

/// Rates probed on every device.
const PROBE_RATES: &[u32] = &[
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
];

macro_rules! alsa_api {
    ($($name:ident: fn($($arg:ty),*) -> $ret:ty;)*) => {
        #[allow(non_snake_case)]
        struct Api {
            _handle: *mut c_void,
            $($name: unsafe extern "C" fn($($arg),*) -> $ret,)*
        }

        impl Api {
            fn load() -> Result<Self, String> {
                // SAFETY: dlopen/dlsym with valid NUL-terminated names; the
                // handle stays open for the process lifetime.
                unsafe {
                    let library = CString::new("libasound.so.2").expect("static name");
                    let handle = libc::dlopen(library.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                    if handle.is_null() {
                        return Err("libasound.so.2 is not installed".to_string());
                    }
                    Ok(Self {
                        _handle: handle,
                        $($name: {
                            let symbol = CString::new(stringify!($name)).expect("static name");
                            let pointer = libc::dlsym(handle, symbol.as_ptr());
                            if pointer.is_null() {
                                return Err(format!("libasound lacks {}", stringify!($name)));
                            }
                            std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) -> $ret>(pointer)
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
    snd_ctl_open: fn(*mut *mut snd_ctl_t, *const c_char, c_int) -> c_int;
    snd_ctl_close: fn(*mut snd_ctl_t) -> c_int;
    snd_ctl_pcm_next_device: fn(*mut snd_ctl_t, *mut c_int) -> c_int;
    snd_ctl_pcm_info: fn(*mut snd_ctl_t, *mut snd_pcm_info_t) -> c_int;
    snd_pcm_info_malloc: fn(*mut *mut snd_pcm_info_t) -> c_int;
    snd_pcm_info_free: fn(*mut snd_pcm_info_t) -> ();
    snd_pcm_info_set_device: fn(*mut snd_pcm_info_t, c_uint) -> ();
    snd_pcm_info_set_subdevice: fn(*mut snd_pcm_info_t, c_uint) -> ();
    snd_pcm_info_set_stream: fn(*mut snd_pcm_info_t, c_int) -> ();
    snd_pcm_info_get_name: fn(*const snd_pcm_info_t) -> *const c_char;
    snd_pcm_open: fn(*mut *mut snd_pcm_t, *const c_char, c_int, c_int) -> c_int;
    snd_pcm_close: fn(*mut snd_pcm_t) -> c_int;
    snd_pcm_nonblock: fn(*mut snd_pcm_t, c_int) -> c_int;
    snd_pcm_hw_params_malloc: fn(*mut *mut snd_pcm_hw_params_t) -> c_int;
    snd_pcm_hw_params_free: fn(*mut snd_pcm_hw_params_t) -> ();
    snd_pcm_hw_params_any: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t) -> c_int;
    snd_pcm_hw_params_test_rate: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_uint, c_int) -> c_int;
    snd_pcm_hw_params_test_format: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_int) -> c_int;
    snd_pcm_hw_params_get_channels_max: fn(*const snd_pcm_hw_params_t, *mut c_uint) -> c_int;
    snd_pcm_hw_params_set_rate_resample: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_uint) -> c_int;
    snd_pcm_hw_params_set_access: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_int) -> c_int;
    snd_pcm_hw_params_set_format: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_int) -> c_int;
    snd_pcm_hw_params_set_channels: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_uint) -> c_int;
    snd_pcm_hw_params_set_rate: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, c_uint, c_int) -> c_int;
    snd_pcm_hw_params_set_buffer_time_near: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, *mut c_uint, *mut c_int) -> c_int;
    snd_pcm_hw_params_set_period_time_near: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t, *mut c_uint, *mut c_int) -> c_int;
    snd_pcm_hw_params_can_pause: fn(*const snd_pcm_hw_params_t) -> c_int;
    snd_pcm_hw_params: fn(*mut snd_pcm_t, *mut snd_pcm_hw_params_t) -> c_int;
    snd_pcm_writei: fn(*mut snd_pcm_t, *const c_void, c_ulong) -> c_long;
    snd_pcm_recover: fn(*mut snd_pcm_t, c_int, c_int) -> c_int;
    snd_pcm_delay: fn(*mut snd_pcm_t, *mut c_long) -> c_int;
    snd_pcm_pause: fn(*mut snd_pcm_t, c_int) -> c_int;
    snd_pcm_drop: fn(*mut snd_pcm_t) -> c_int;
    snd_pcm_drain: fn(*mut snd_pcm_t) -> c_int;
    snd_pcm_prepare: fn(*mut snd_pcm_t) -> c_int;
    snd_mixer_open: fn(*mut *mut snd_mixer_t, c_int) -> c_int;
    snd_mixer_attach: fn(*mut snd_mixer_t, *const c_char) -> c_int;
    snd_mixer_selem_register: fn(*mut snd_mixer_t, *mut c_void, *mut *mut c_void) -> c_int;
    snd_mixer_load: fn(*mut snd_mixer_t) -> c_int;
    snd_mixer_close: fn(*mut snd_mixer_t) -> c_int;
    snd_mixer_first_elem: fn(*mut snd_mixer_t) -> *mut snd_mixer_elem_t;
    snd_mixer_elem_next: fn(*mut snd_mixer_elem_t) -> *mut snd_mixer_elem_t;
    snd_mixer_selem_get_name: fn(*mut snd_mixer_elem_t) -> *const c_char;
    snd_mixer_selem_has_playback_volume: fn(*mut snd_mixer_elem_t) -> c_int;
    snd_mixer_selem_get_playback_dB_range: fn(*mut snd_mixer_elem_t, *mut c_long, *mut c_long) -> c_int;
    snd_mixer_selem_set_playback_dB_all: fn(*mut snd_mixer_elem_t, c_long, c_int) -> c_int;
    snd_mixer_selem_has_playback_switch: fn(*mut snd_mixer_elem_t) -> c_int;
    snd_mixer_selem_set_playback_switch_all: fn(*mut snd_mixer_elem_t, c_int) -> c_int;
    snd_strerror: fn(c_int) -> *const c_char;
}

// SAFETY: the function pointers are plain C entry points; libasound is
// thread-safe for distinct handles, and each PCM handle is only used by
// the one playback thread that owns its stream.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn c_text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: ALSA returns NUL-terminated strings.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .trim()
        .to_string()
}

fn format_code(format: SampleFormat) -> c_int {
    match format {
        SampleFormat::S16 => SND_PCM_FORMAT_S16_LE,
        SampleFormat::S24Packed => SND_PCM_FORMAT_S24_3LE,
        SampleFormat::S32 => SND_PCM_FORMAT_S32_LE,
    }
}

/// Card and device numbers behind a device id.
#[derive(Clone, Debug)]
struct Address {
    card: c_int,
    device: c_int,
    /// Explicit PCM name (tests: `null`, `file:...`).
    pcm: Option<String>,
}

pub struct AlsaBackend {
    api: std::sync::Arc<Api>,
    addresses: Mutex<std::collections::HashMap<String, Address>>,
    /// Extra PCMs by name (for tests and unusual setups).
    extra: Vec<(String, String)>,
    /// Last good probe per device, reused while another app holds it.
    probes: Mutex<std::collections::HashMap<String, DeviceInfo>>,
    /// Devices Parson itself has open (busy because of us, not others).
    open: std::sync::Arc<Mutex<std::collections::HashSet<String>>>,
}

impl AlsaBackend {
    pub fn load() -> Result<Self, String> {
        Ok(Self {
            api: std::sync::Arc::new(Api::load()?),
            addresses: Mutex::new(std::collections::HashMap::new()),
            extra: Vec::new(),
            probes: Mutex::new(std::collections::HashMap::new()),
            open: std::sync::Arc::new(Mutex::new(std::collections::HashSet::new())),
        })
    }

    /// Adds a PCM by name as a device (e.g. ALSA's `null` plugin).
    pub fn with_pcm(mut self, id: &str, pcm: &str) -> Self {
        self.extra.push((id.to_string(), pcm.to_string()));
        self
    }

    fn error(&self, code: c_int) -> String {
        // SAFETY: snd_strerror returns a static string.
        c_text(unsafe { (self.api.snd_strerror)(code) })
    }

    /// Opens `name` and reads what it takes.
    fn probe(&self, name: &str) -> Result<(Vec<u32>, Vec<SampleFormat>, u8), c_int> {
        let api = &self.api;
        let pcm_name = CString::new(name).map_err(|_| -22)?;
        let mut pcm: *mut snd_pcm_t = std::ptr::null_mut();
        // SAFETY: valid out-pointers; every opened handle is closed below.
        unsafe {
            let opened = (api.snd_pcm_open)(
                &mut pcm,
                pcm_name.as_ptr(),
                SND_PCM_STREAM_PLAYBACK,
                SND_PCM_NONBLOCK,
            );
            if opened < 0 {
                return Err(opened);
            }
            let mut params: *mut snd_pcm_hw_params_t = std::ptr::null_mut();
            if (api.snd_pcm_hw_params_malloc)(&mut params) < 0 {
                (api.snd_pcm_close)(pcm);
                return Err(-12);
            }
            (api.snd_pcm_hw_params_any)(pcm, params);
            // Probe the hardware's own rates, not what a plugin could convert.
            (api.snd_pcm_hw_params_set_rate_resample)(pcm, params, 0);
            let rates = PROBE_RATES
                .iter()
                .copied()
                .filter(|rate| (api.snd_pcm_hw_params_test_rate)(pcm, params, *rate, 0) == 0)
                .collect();
            let formats = [
                SampleFormat::S16,
                SampleFormat::S24Packed,
                SampleFormat::S32,
            ]
            .into_iter()
            .filter(|format| {
                (api.snd_pcm_hw_params_test_format)(pcm, params, format_code(*format)) == 0
            })
            .collect();
            let mut channels: c_uint = 2;
            (api.snd_pcm_hw_params_get_channels_max)(params, &mut channels);
            (api.snd_pcm_hw_params_free)(params);
            (api.snd_pcm_close)(pcm);
            Ok((rates, formats, channels.clamp(1, 32) as u8))
        }
    }

    /// The card's playback mixer element with a dB range, if any.
    fn with_mixer<T>(
        &self,
        card: c_int,
        action: impl FnOnce(&Api, *mut snd_mixer_elem_t) -> T,
    ) -> Option<T> {
        let api = &self.api;
        let name = CString::new(format!("hw:{card}")).ok()?;
        let mut mixer: *mut snd_mixer_t = std::ptr::null_mut();
        // SAFETY: the mixer is opened, used and closed in this scope.
        unsafe {
            if (api.snd_mixer_open)(&mut mixer, 0) < 0 {
                return None;
            }
            let ready = (api.snd_mixer_attach)(mixer, name.as_ptr()) >= 0
                && (api.snd_mixer_selem_register)(
                    mixer,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) >= 0
                && (api.snd_mixer_load)(mixer) >= 0;
            let mut chosen: *mut snd_mixer_elem_t = std::ptr::null_mut();
            if ready {
                let mut element = (api.snd_mixer_first_elem)(mixer);
                let mut rank = usize::MAX;
                while !element.is_null() {
                    if (api.snd_mixer_selem_has_playback_volume)(element) != 0 {
                        let mut minimum: c_long = 0;
                        let mut maximum: c_long = 0;
                        let has_db = (api.snd_mixer_selem_get_playback_dB_range)(
                            element,
                            &mut minimum,
                            &mut maximum,
                        ) == 0
                            && maximum > minimum;
                        let element_name = c_text((api.snd_mixer_selem_get_name)(element));
                        let order = ["PCM", "Master", "Digital", "Speaker", "Headphone"]
                            .iter()
                            .position(|candidate| element_name.eq_ignore_ascii_case(candidate))
                            .unwrap_or(10);
                        if has_db && order < rank {
                            rank = order;
                            chosen = element;
                        }
                    }
                    element = (api.snd_mixer_elem_next)(element);
                }
            }
            let result = (!chosen.is_null()).then(|| action(api, chosen));
            (api.snd_mixer_close)(mixer);
            result
        }
    }

    fn hardware_volume(&self, card: c_int) -> Option<HardwareVolume> {
        self.with_mixer(card, |api, element| {
            let mut minimum: c_long = 0;
            let mut maximum: c_long = 0;
            // SAFETY: element comes from the open mixer.
            unsafe {
                (api.snd_mixer_selem_get_playback_dB_range)(element, &mut minimum, &mut maximum);
            }
            HardwareVolume {
                min_db: minimum as f32 / 100.0,
                max_db: maximum as f32 / 100.0,
                step_db: 0.5,
            }
        })
    }

    fn address(&self, device_id: &str) -> Option<Address> {
        self.addresses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(device_id)
            .cloned()
    }
}

fn sanitize(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    cleaned.trim_matches('-').chars().take(40).collect()
}

impl DeviceBackend for AlsaBackend {
    fn name(&self) -> &'static str {
        "ALSA"
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let api = &self.api;
        let mut devices = Vec::new();
        let mut addresses = std::collections::HashMap::new();
        let mut card: c_int = -1;
        // SAFETY: standard ALSA enumeration; every handle is closed.
        unsafe {
            while (api.snd_card_next)(&mut card) == 0 && card >= 0 {
                let mut short: *mut c_char = std::ptr::null_mut();
                let mut long: *mut c_char = std::ptr::null_mut();
                (api.snd_card_get_name)(card, &mut short);
                (api.snd_card_get_longname)(card, &mut long);
                let card_name = c_text(short);
                let long_name = c_text(long);
                libc::free(short.cast());
                libc::free(long.cast());
                let Ok(control_name) = CString::new(format!("hw:{card}")) else {
                    continue;
                };
                let mut control: *mut snd_ctl_t = std::ptr::null_mut();
                if (api.snd_ctl_open)(&mut control, control_name.as_ptr(), 0) < 0 {
                    continue;
                }
                let mut info: *mut snd_pcm_info_t = std::ptr::null_mut();
                if (api.snd_pcm_info_malloc)(&mut info) < 0 {
                    (api.snd_ctl_close)(control);
                    continue;
                }
                let mut device: c_int = -1;
                while (api.snd_ctl_pcm_next_device)(control, &mut device) == 0 && device >= 0 {
                    (api.snd_pcm_info_set_device)(info, device as c_uint);
                    (api.snd_pcm_info_set_subdevice)(info, 0);
                    (api.snd_pcm_info_set_stream)(info, SND_PCM_STREAM_PLAYBACK);
                    if (api.snd_ctl_pcm_info)(control, info) < 0 {
                        continue; // capture-only
                    }
                    let pcm_name = c_text((api.snd_pcm_info_get_name)(info));
                    let id = format!("alsa-{}-{device}", sanitize(&card_name));
                    let transport = if long_name.contains("USB") {
                        "USB"
                    } else if pcm_name.contains("HDMI") || pcm_name.contains("DisplayPort") {
                        "HDMI"
                    } else {
                        "Built-in"
                    };
                    let hw = format!("hw:{card},{device}");
                    let ours = self
                        .open
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .contains(&id);
                    // Never probe a device Parson is playing on.
                    let probe = if ours { Err(-EBUSY) } else { self.probe(&hw) };
                    let busy = !ours && matches!(probe, Err(code) if code == -EBUSY);
                    let previous = self
                        .probes
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .get(&id)
                        .cloned();
                    let (sample_rates, formats, max_channels) = match probe {
                        Ok(probed) => probed,
                        Err(_) => match &previous {
                            Some(known) => (
                                known.sample_rates.clone(),
                                known.formats.clone(),
                                known.max_channels,
                            ),
                            None => (vec![44_100, 48_000], vec![SampleFormat::S16], 2),
                        },
                    };
                    if formats.is_empty() || sample_rates.is_empty() {
                        continue;
                    }
                    let name = if pcm_name.is_empty() || transport == "USB" {
                        card_name.clone()
                    } else {
                        format!("{card_name} {pcm_name}")
                    };
                    let info = DeviceInfo {
                        id: id.clone(),
                        name,
                        description: format!("{transport} audio · {hw}"),
                        sample_rates,
                        formats,
                        max_channels,
                        hardware_volume: self.hardware_volume(card),
                        busy,
                    };
                    self.probes
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(id.clone(), info.clone());
                    addresses.insert(
                        id,
                        Address {
                            card,
                            device,
                            pcm: None,
                        },
                    );
                    devices.push(info);
                }
                (api.snd_pcm_info_free)(info);
                (api.snd_ctl_close)(control);
            }
        }
        for (id, pcm) in &self.extra {
            if let Ok((sample_rates, formats, max_channels)) = self.probe(pcm) {
                addresses.insert(
                    id.clone(),
                    Address {
                        card: -1,
                        device: -1,
                        pcm: Some(pcm.clone()),
                    },
                );
                devices.push(DeviceInfo {
                    id: id.clone(),
                    name: format!("ALSA {pcm}"),
                    description: format!("ALSA PCM · {pcm}"),
                    sample_rates: if sample_rates.is_empty() {
                        vec![44_100, 48_000, 96_000]
                    } else {
                        sample_rates
                    },
                    formats: if formats.is_empty() {
                        vec![SampleFormat::S16]
                    } else {
                        formats
                    },
                    max_channels,
                    hardware_volume: None,
                    busy: false,
                });
            }
        }
        *self
            .addresses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = addresses;
        devices
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> Result<Box<dyn OutputStream>, String> {
        let address = self
            .address(device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        let name = match &address.pcm {
            Some(pcm) => pcm.clone(),
            None if exclusive => format!("hw:{},{}", address.card, address.device),
            None => format!("plughw:{},{}", address.card, address.device),
        };
        let api = self.api.clone();
        let pcm_name = CString::new(name.clone()).map_err(|error| error.to_string())?;
        let mut pcm: *mut snd_pcm_t = std::ptr::null_mut();
        // SAFETY: the handle is owned by the returned stream, which closes it.
        unsafe {
            let opened = (api.snd_pcm_open)(
                &mut pcm,
                pcm_name.as_ptr(),
                SND_PCM_STREAM_PLAYBACK,
                SND_PCM_NONBLOCK,
            );
            if opened < 0 {
                return Err(if opened == -EBUSY {
                    format!("{name} is in use by another app")
                } else {
                    format!("cannot open {name}: {}", self.error(opened))
                });
            }
            // Blocking writes from here on.
            (api.snd_pcm_nonblock)(pcm, 0);
            let mut params: *mut snd_pcm_hw_params_t = std::ptr::null_mut();
            if (api.snd_pcm_hw_params_malloc)(&mut params) < 0 {
                (api.snd_pcm_close)(pcm);
                return Err("out of memory".to_string());
            }
            let fail = |code: c_int, what: &str| -> String {
                format!("{name}: {what} failed: {}", self.error(code))
            };
            let mut error = None;
            let result = (api.snd_pcm_hw_params_any)(pcm, params);
            if result < 0 {
                error = Some(fail(result, "reading parameters"));
            }
            // Exclusive: the hardware's own rate or nothing (no resampling).
            let resample = c_uint::from(!exclusive);
            let result = (api.snd_pcm_hw_params_set_rate_resample)(pcm, params, resample);
            if error.is_none() && result < 0 {
                error = Some(fail(result, "disabling resampling"));
            }
            let result =
                (api.snd_pcm_hw_params_set_access)(pcm, params, SND_PCM_ACCESS_RW_INTERLEAVED);
            if error.is_none() && result < 0 {
                error = Some(fail(result, "interleaved access"));
            }
            let result =
                (api.snd_pcm_hw_params_set_format)(pcm, params, format_code(format.format));
            if error.is_none() && result < 0 {
                error = Some(fail(result, "sample format"));
            }
            let result =
                (api.snd_pcm_hw_params_set_channels)(pcm, params, c_uint::from(format.channels));
            if error.is_none() && result < 0 {
                error = Some(fail(result, "channel count"));
            }
            let result = (api.snd_pcm_hw_params_set_rate)(pcm, params, format.sample_rate, 0);
            if error.is_none() && result < 0 {
                error = Some(fail(result, "sample rate"));
            }
            if error.is_none() {
                let mut buffer_us: c_uint = 200_000;
                let mut period_us: c_uint = 20_000;
                (api.snd_pcm_hw_params_set_buffer_time_near)(
                    pcm,
                    params,
                    &mut buffer_us,
                    std::ptr::null_mut(),
                );
                (api.snd_pcm_hw_params_set_period_time_near)(
                    pcm,
                    params,
                    &mut period_us,
                    std::ptr::null_mut(),
                );
                let result = (api.snd_pcm_hw_params)(pcm, params);
                if result < 0 {
                    error = Some(fail(result, "applying parameters"));
                }
            }
            let can_pause = error.is_none() && (api.snd_pcm_hw_params_can_pause)(params) == 1;
            (api.snd_pcm_hw_params_free)(params);
            if let Some(error) = error {
                (api.snd_pcm_close)(pcm);
                return Err(error);
            }
            self.open
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(device_id.to_string());
            Ok(Box::new(AlsaStream {
                api,
                pcm,
                format,
                can_pause,
                device_id: device_id.to_string(),
                open: self.open.clone(),
            }))
        }
    }

    fn set_hardware_volume(&self, device_id: &str, db: f32) -> Result<(), String> {
        let address = self
            .address(device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        self.with_mixer(address.card, |api, element| {
            // SAFETY: element comes from the open mixer.
            unsafe {
                (api.snd_mixer_selem_set_playback_dB_all)(
                    element,
                    (db * 100.0).round() as c_long,
                    0,
                )
            }
        })
        .filter(|code| *code >= 0)
        .map(|_| ())
        .ok_or_else(|| "the device has no volume control".to_string())
    }

    fn set_hardware_mute(&self, device_id: &str, muted: bool) -> Result<(), String> {
        let address = self
            .address(device_id)
            .ok_or_else(|| "device unplugged".to_string())?;
        self.with_mixer(address.card, |api, element| {
            // SAFETY: element comes from the open mixer.
            unsafe {
                if (api.snd_mixer_selem_has_playback_switch)(element) == 0 {
                    return -1;
                }
                (api.snd_mixer_selem_set_playback_switch_all)(element, if muted { 0 } else { 1 })
            }
        })
        .filter(|code| *code >= 0)
        .map(|_| ())
        .ok_or_else(|| "the device cannot mute itself".to_string())
    }
}

struct AlsaStream {
    api: std::sync::Arc<Api>,
    pcm: *mut snd_pcm_t,
    format: StreamFormat,
    can_pause: bool,
    device_id: String,
    open: std::sync::Arc<Mutex<std::collections::HashSet<String>>>,
}

// SAFETY: the PCM handle is only used by the thread that owns the stream.
unsafe impl Send for AlsaStream {}

impl OutputStream for AlsaStream {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        let frame_bytes = self.format.frame_bytes();
        let mut offset = 0;
        while offset < data.len() {
            let frames = (data.len() - offset) / frame_bytes;
            // SAFETY: the pointer and frame count stay inside `data`.
            let written = unsafe {
                (self.api.snd_pcm_writei)(
                    self.pcm,
                    data[offset..].as_ptr().cast(),
                    frames as c_ulong,
                )
            };
            if written < 0 {
                let code = written as c_int;
                // SAFETY: recover from an underrun (EPIPE) or suspend.
                let recovered = unsafe { (self.api.snd_pcm_recover)(self.pcm, code, 1) };
                if recovered < 0 && code != -EPIPE {
                    // SAFETY: static string.
                    return Err(c_text(unsafe { (self.api.snd_strerror)(code) }));
                }
                continue;
            }
            offset += written as usize * frame_bytes;
        }
        Ok(())
    }

    fn delay_frames(&mut self) -> u64 {
        let mut delay: c_long = 0;
        // SAFETY: valid handle and out-pointer.
        if unsafe { (self.api.snd_pcm_delay)(self.pcm, &mut delay) } < 0 {
            return 0;
        }
        delay.max(0) as u64
    }

    fn pause(&mut self, paused: bool) -> Result<(), String> {
        if !self.can_pause {
            return Err("the device cannot pause".to_string());
        }
        // SAFETY: valid handle.
        let result = unsafe { (self.api.snd_pcm_pause)(self.pcm, c_int::from(paused)) };
        if result < 0 {
            Err("pause failed".to_string())
        } else {
            Ok(())
        }
    }

    fn drop_buffer(&mut self) {
        // SAFETY: valid handle; prepare makes it writable again.
        unsafe {
            (self.api.snd_pcm_drop)(self.pcm);
            (self.api.snd_pcm_prepare)(self.pcm);
        }
    }

    fn drain(&mut self) {
        // SAFETY: valid handle.
        unsafe {
            (self.api.snd_pcm_drain)(self.pcm);
        }
    }
}

impl Drop for AlsaStream {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by this stream and is closed once.
        unsafe {
            (self.api.snd_pcm_close)(self.pcm);
        }
        self.open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.device_id);
    }
}

#[cfg(test)]
mod tests {
    use super::AlsaBackend;
    use crate::audio::{DeviceBackend, SampleFormat, StreamFormat};

    /// The real ALSA write path against ALSA's `null` PCM (no sound card
    /// needed): open with exact parameters, write, pause, drain, close.
    #[test]
    fn writes_through_libasound_when_it_is_installed() {
        let Ok(backend) = AlsaBackend::load() else {
            eprintln!("libasound.so.2 not installed; skipping");
            return;
        };
        let backend = backend.with_pcm("alsa-null", "null");
        let devices = backend.devices();
        let Some(null) = devices.iter().find(|device| device.id == "alsa-null") else {
            eprintln!("ALSA null PCM unavailable; skipping");
            return;
        };
        assert!(!null.formats.is_empty());
        let format = StreamFormat {
            sample_rate: 96_000,
            format: SampleFormat::S32,
            channels: 2,
        };
        let mut stream = backend
            .open("alsa-null", format, true)
            .expect("open null PCM");
        let second = vec![0_u8; 96_000 * format.frame_bytes() / 10];
        for _ in 0..5 {
            stream.write(&second).expect("write");
        }
        let _ = stream.pause(true);
        let _ = stream.pause(false);
        stream.drop_buffer();
        stream.write(&second).expect("write after drop");
        stream.drain();
        assert!(backend.open("missing", format, true).is_err());
    }
}
