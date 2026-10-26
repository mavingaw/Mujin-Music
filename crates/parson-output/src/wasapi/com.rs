//! The real WASAPI, through COM: `IMMDeviceEnumerator` for endpoints and
//! their names, `IAudioClient` / `IAudioRenderClient` / `IAudioClock` for
//! streams, `IAudioEndpointVolume` for hardware volume in dB. Hand-written
//! vtables (only the methods Parson calls are typed) keep the bridge free of
//! large Windows binding crates. Every call runs on a COM multithreaded
//! apartment thread; the objects involved are free-threaded.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::ptr::{NonNull, null, null_mut};

use super::{
    E_NOTFOUND, Endpoint, EndpointVolume, HResult, ShareMode, WasapiApi, WasapiBackend,
    WasapiClient, WaveFormat,
};

type HRESULT = i32;
type HANDLE = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct GUID {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GUID {
    GUID {
        data1,
        data2,
        data3,
        data4,
    }
}

const CLSID_MMDEVICE_ENUMERATOR: GUID = guid(
    0xBCDE_0395,
    0xE52F,
    0x467C,
    [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
);
const IID_IMMDEVICE_ENUMERATOR: GUID = guid(
    0xA956_64D2,
    0x9614,
    0x4F35,
    [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
);
const IID_IAUDIO_CLIENT: GUID = guid(
    0x1CB9_AD4C,
    0xDBFA,
    0x4C32,
    [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
);
const IID_IAUDIO_RENDER_CLIENT: GUID = guid(
    0xF294_ACFC,
    0x3146,
    0x4483,
    [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2],
);
const IID_IAUDIO_CLOCK: GUID = guid(
    0xCD63_314F,
    0x3FBA,
    0x4A1B,
    [0x81, 0x2C, 0xEF, 0x96, 0x35, 0x87, 0x28, 0xE7],
);
const IID_IAUDIO_ENDPOINT_VOLUME: GUID = guid(
    0x5CDF_2C82,
    0x841E,
    0x4546,
    [0x97, 0x22, 0x0C, 0xF7, 0x40, 0x78, 0x22, 0x9A],
);
const KSDATAFORMAT_SUBTYPE_PCM: GUID = guid(
    0x0000_0001,
    0x0000,
    0x0010,
    [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
);

#[repr(C)]
struct PROPERTYKEY {
    fmtid: GUID,
    pid: u32,
}

const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: guid(
        0xA45C_254E,
        0xDF1C,
        0x4EFD,
        [0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0],
    ),
    pid: 14,
};
const PKEY_DEVICE_INTERFACE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: guid(
        0x026E_516E,
        0xB814,
        0x414B,
        [0x83, 0xCD, 0x85, 0x6D, 0x6F, 0xEF, 0x48, 0x22],
    ),
    pid: 2,
};

/// `PROPVARIANT`: the type tag, then a 16-byte union (8 on 32-bit).
#[repr(C)]
struct PROPVARIANT {
    vt: u16,
    reserved: [u16; 3],
    data: [usize; 2],
}

const VT_LPWSTR: u16 = 31;
const CLSCTX_ALL: u32 = 0x17;
const COINIT_MULTITHREADED: u32 = 0;
const E_RENDER: i32 = 0;
const DEVICE_STATE_ACTIVE: u32 = 1;
const STGM_READ: u32 = 0;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const AUDCLNT_SHAREMODE_SHARED: i32 = 0;
const AUDCLNT_SHAREMODE_EXCLUSIVE: i32 = 1;
const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 0x0004_0000;
const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;
const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;
const ENDPOINT_HARDWARE_SUPPORT_VOLUME: u32 = 0x1;
const WAIT_OBJECT_0: u32 = 0;

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WAVEFORMATEX {
    wFormatTag: u16,
    nChannels: u16,
    nSamplesPerSec: u32,
    nAvgBytesPerSec: u32,
    nBlockAlign: u16,
    wBitsPerSample: u16,
    cbSize: u16,
}

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WAVEFORMATEXTENSIBLE {
    Format: WAVEFORMATEX,
    wValidBitsPerSample: u16,
    dwChannelMask: u32,
    SubFormat: GUID,
}

fn channel_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        4 => 0x33,
        6 => 0x3F,
        8 => 0x63F,
        other => (1_u32 << other.min(31)) - 1,
    }
}

fn wave_format(format: &WaveFormat) -> WAVEFORMATEXTENSIBLE {
    let block_align = format.block_align() as u16;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: format.channels,
            nSamplesPerSec: format.sample_rate,
            nAvgBytesPerSec: format.sample_rate * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: format.container_bits,
            cbSize: 22,
        },
        wValidBitsPerSample: format.valid_bits,
        dwChannelMask: channel_mask(format.channels),
        SubFormat: KSDATAFORMAT_SUBTYPE_PCM,
    }
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const c_void, coinit: u32) -> HRESULT;
    fn CoCreateInstance(
        clsid: *const GUID,
        outer: *mut c_void,
        context: u32,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT;
    fn CoTaskMemFree(pointer: *const c_void);
    fn PropVariantClear(value: *mut PROPVARIANT) -> HRESULT;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateEventW(
        attributes: *const c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> i32;
}

type Unused = usize;

#[repr(C)]
struct IUnknownVtbl {
    QueryInterface: Unused,
    AddRef: Unused,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IMMDeviceEnumeratorVtbl {
    base: IUnknownVtbl,
    EnumAudioEndpoints:
        unsafe extern "system" fn(*mut c_void, i32, u32, *mut *mut c_void) -> HRESULT,
    GetDefaultAudioEndpoint: Unused,
    GetDevice: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT,
    RegisterEndpointNotificationCallback: Unused,
    UnregisterEndpointNotificationCallback: Unused,
}

#[repr(C)]
struct IMMDeviceCollectionVtbl {
    base: IUnknownVtbl,
    GetCount: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    Item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct IMMDeviceVtbl {
    base: IUnknownVtbl,
    Activate: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        u32,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    OpenPropertyStore: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    GetId: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HRESULT,
    GetState: Unused,
}

#[repr(C)]
struct IPropertyStoreVtbl {
    base: IUnknownVtbl,
    GetCount: Unused,
    GetAt: Unused,
    GetValue:
        unsafe extern "system" fn(*mut c_void, *const PROPERTYKEY, *mut PROPVARIANT) -> HRESULT,
    SetValue: Unused,
    Commit: Unused,
}

#[repr(C)]
struct IAudioClientVtbl {
    base: IUnknownVtbl,
    Initialize: unsafe extern "system" fn(
        *mut c_void,
        i32,
        u32,
        i64,
        i64,
        *const c_void,
        *const GUID,
    ) -> HRESULT,
    GetBufferSize: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    GetStreamLatency: Unused,
    GetCurrentPadding: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    IsFormatSupported:
        unsafe extern "system" fn(*mut c_void, i32, *const c_void, *mut *mut c_void) -> HRESULT,
    GetMixFormat: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    GetDevicePeriod: unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> HRESULT,
    Start: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    Stop: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    Reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    SetEventHandle: unsafe extern "system" fn(*mut c_void, HANDLE) -> HRESULT,
    GetService: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct IAudioRenderClientVtbl {
    base: IUnknownVtbl,
    GetBuffer: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> HRESULT,
    ReleaseBuffer: unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT,
}

#[repr(C)]
struct IAudioClockVtbl {
    base: IUnknownVtbl,
    GetFrequency: unsafe extern "system" fn(*mut c_void, *mut u64) -> HRESULT,
    GetPosition: unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64) -> HRESULT,
    GetCharacteristics: Unused,
}

#[repr(C)]
struct IAudioEndpointVolumeVtbl {
    base: IUnknownVtbl,
    RegisterControlChangeNotify: Unused,
    UnregisterControlChangeNotify: Unused,
    GetChannelCount: Unused,
    SetMasterVolumeLevel: unsafe extern "system" fn(*mut c_void, f32, *const GUID) -> HRESULT,
    SetMasterVolumeLevelScalar: Unused,
    GetMasterVolumeLevel: Unused,
    GetMasterVolumeLevelScalar: Unused,
    SetChannelVolumeLevel: Unused,
    SetChannelVolumeLevelScalar: Unused,
    GetChannelVolumeLevel: Unused,
    GetChannelVolumeLevelScalar: Unused,
    SetMute: unsafe extern "system" fn(*mut c_void, i32, *const GUID) -> HRESULT,
    GetMute: Unused,
    GetVolumeStepInfo: Unused,
    VolumeStepUp: Unused,
    VolumeStepDown: Unused,
    QueryHardwareSupport: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    GetVolumeRange: unsafe extern "system" fn(*mut c_void, *mut f32, *mut f32, *mut f32) -> HRESULT,
}

/// An owned, non-null COM interface pointer: it holds one reference,
/// released exactly once when it drops, so the object stays alive (and its
/// vtable readable) for as long as the `Com` exists.
struct Com(NonNull<c_void>);

// SAFETY: the MMDevice and audio-client objects are free-threaded, and
// every thread that touches them has joined the multithreaded apartment.
unsafe impl Send for Com {}
unsafe impl Sync for Com {}

impl Com {
    /// Takes ownership of an interface pointer a COM call returned. A call
    /// that succeeds without setting its out pointer is refused here
    /// (`E_POINTER`) rather than dereferenced later.
    fn from_raw(pointer: *mut c_void) -> Result<Self, HResult> {
        NonNull::new(pointer).map(Self).ok_or(E_POINTER)
    }

    fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }

    /// The object's vtable as `T`.
    ///
    /// # Safety
    /// `T` must be the vtable layout of the interface this pointer holds.
    unsafe fn vtable<T>(&self) -> &T {
        // SAFETY: the pointer is non-null and owns a live reference (see
        // the type), and a COM object starts with a pointer to its vtable,
        // which lives as long as the object.
        unsafe { &**(self.as_ptr() as *const *const T) }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: every interface starts with IUnknown; this releases the
        // one reference `from_raw` took over.
        unsafe { (self.vtable::<IUnknownVtbl>().Release)(self.as_ptr()) };
    }
}

/// `E_POINTER`: a call returned success without its out pointer.
const E_POINTER: HResult = 0x8000_4003_u32 as i32;
/// Longest wide string read from a COM out parameter (device ids and
/// names are far shorter); a missing terminator stops here.
const MAX_WIDE_CHARS: usize = 32 * 1024;

/// Length of a NUL-terminated UTF-16 string, at most [`MAX_WIDE_CHARS`].
///
/// # Safety
/// `pointer` must be non-null and point to a NUL-terminated UTF-16 string
/// (or to at least `MAX_WIDE_CHARS` readable units).
unsafe fn wide_length(pointer: *const u16) -> usize {
    let mut length = 0;
    // SAFETY: the caller guarantees the units up to the terminator (or the
    // cap) are readable.
    while length < MAX_WIDE_CHARS && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    length
}

fn check(hr: HRESULT) -> Result<(), HResult> {
    if hr < 0 { Err(hr) } else { Ok(()) }
}

/// Joins this thread to the multithreaded apartment, once.
fn ensure_com() {
    thread_local! {
        static JOINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    JOINED.with(|joined| {
        if !joined.get() {
            // SAFETY: plain call; S_FALSE (already joined) and
            // RPC_E_CHANGED_MODE (an STA thread) both leave COM usable for
            // these free-threaded objects.
            unsafe { CoInitializeEx(null(), COINIT_MULTITHREADED) };
            joined.set(true);
        }
    });
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads and frees a CoTaskMem wide string.
///
/// # Safety
/// `pointer` is null or a NUL-terminated string from CoTaskMemAlloc.
unsafe fn take_wide(pointer: *mut u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: NUL-terminated per the contract; the slice stops before the
    // terminator.
    let text = unsafe {
        let length = wide_length(pointer);
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    };
    // SAFETY: allocated by COM for us to free.
    unsafe { CoTaskMemFree(pointer.cast()) };
    text
}

pub struct ComWasapi {
    enumerator: Com,
}

/// The machine's WASAPI.
pub fn load() -> Result<WasapiBackend<ComWasapi>, String> {
    ensure_com();
    let mut enumerator = null_mut();
    // SAFETY: valid GUIDs and out pointer.
    let hr = unsafe {
        CoCreateInstance(
            &CLSID_MMDEVICE_ENUMERATOR,
            null_mut(),
            CLSCTX_ALL,
            &IID_IMMDEVICE_ENUMERATOR,
            &mut enumerator,
        )
    };
    if hr < 0 || enumerator.is_null() {
        return Err(format!("WASAPI is unavailable: {}", super::describe(hr)));
    }
    Ok(WasapiBackend::new(ComWasapi {
        enumerator: Com::from_raw(enumerator).map_err(super::describe)?,
    }))
}

impl ComWasapi {
    fn device(&self, id: &str) -> Result<Com, HResult> {
        ensure_com();
        let id = wide(id);
        let mut device = null_mut();
        // SAFETY: the enumerator vtable, a NUL-terminated id, an out pointer.
        let hr = unsafe {
            (self
                .enumerator
                .vtable::<IMMDeviceEnumeratorVtbl>()
                .GetDevice)(self.enumerator.as_ptr(), id.as_ptr(), &mut device)
        };
        check(hr)?;
        Com::from_raw(device).map_err(|_| E_NOTFOUND)
    }

    fn activate_on(device: &Com, iid: &GUID) -> Result<Com, HResult> {
        let mut object = null_mut();
        // SAFETY: an IMMDevice, a valid IID, no activation parameters.
        let hr = unsafe {
            (device.vtable::<IMMDeviceVtbl>().Activate)(
                device.as_ptr(),
                iid,
                CLSCTX_ALL,
                null(),
                &mut object,
            )
        };
        check(hr)?;
        Com::from_raw(object)
    }

    fn string_property(store: &Com, key: &PROPERTYKEY) -> String {
        let mut value = PROPVARIANT {
            vt: 0,
            reserved: [0; 3],
            data: [0; 2],
        };
        // SAFETY: an IPropertyStore, a valid key, an out PROPVARIANT.
        let hr = unsafe {
            (store.vtable::<IPropertyStoreVtbl>().GetValue)(store.as_ptr(), key, &mut value)
        };
        let mut text = String::new();
        if hr >= 0 && value.vt == VT_LPWSTR && value.data[0] != 0 {
            let pointer = value.data[0] as *const u16;
            // SAFETY: a VT_LPWSTR holds a non-null (checked above),
            // NUL-terminated wide string.
            text = unsafe {
                let length = wide_length(pointer);
                String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
            };
        }
        // SAFETY: frees what GetValue allocated.
        unsafe { PropVariantClear(&mut value) };
        text
    }

    fn mix_format(client: &Com) -> Option<WaveFormat> {
        let mut pointer: *mut c_void = null_mut();
        // SAFETY: an IAudioClient and an out pointer.
        let hr = unsafe {
            (client.vtable::<IAudioClientVtbl>().GetMixFormat)(client.as_ptr(), &mut pointer)
        };
        if hr < 0 || pointer.is_null() {
            return None;
        }
        // SAFETY: the pointer is non-null (checked above) and GetMixFormat
        // returns at least a WAVEFORMATEX, which is read before it is freed.
        let format = unsafe { std::ptr::read_unaligned(pointer as *const WAVEFORMATEX) };
        let extensible = size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>();
        let valid_bits = if format.wFormatTag == WAVE_FORMAT_EXTENSIBLE
            && usize::from(format.cbSize) >= extensible
        {
            // SAFETY: `cbSize` says the extra fields of an extensible
            // format follow the header, so the whole struct is readable.
            unsafe { std::ptr::read_unaligned(pointer as *const WAVEFORMATEXTENSIBLE) }
                .wValidBitsPerSample
        } else {
            format.wBitsPerSample
        };
        // SAFETY: allocated by COM for us to free.
        unsafe { CoTaskMemFree(pointer) };
        Some(WaveFormat {
            sample_rate: format.nSamplesPerSec,
            channels: format.nChannels,
            container_bits: format.wBitsPerSample,
            valid_bits,
        })
    }
}

impl WasapiApi for ComWasapi {
    type Client = ComClient;

    fn endpoints(&self) -> Result<Vec<Endpoint>, HResult> {
        ensure_com();
        let mut collection = null_mut();
        // SAFETY: the enumerator vtable and an out pointer.
        let hr = unsafe {
            (self
                .enumerator
                .vtable::<IMMDeviceEnumeratorVtbl>()
                .EnumAudioEndpoints)(
                self.enumerator.as_ptr(),
                E_RENDER,
                DEVICE_STATE_ACTIVE,
                &mut collection,
            )
        };
        check(hr)?;
        let collection = Com::from_raw(collection)?;
        let mut count = 0;
        // SAFETY: an IMMDeviceCollection.
        check(unsafe {
            (collection.vtable::<IMMDeviceCollectionVtbl>().GetCount)(
                collection.as_ptr(),
                &mut count,
            )
        })?;
        let mut endpoints = Vec::new();
        for index in 0..count {
            let mut device = null_mut();
            // SAFETY: index < count.
            let hr = unsafe {
                (collection.vtable::<IMMDeviceCollectionVtbl>().Item)(
                    collection.as_ptr(),
                    index,
                    &mut device,
                )
            };
            if hr < 0 {
                continue;
            }
            let Ok(device) = Com::from_raw(device) else {
                continue;
            };
            let mut id = null_mut();
            // SAFETY: an IMMDevice and an out string.
            if unsafe { (device.vtable::<IMMDeviceVtbl>().GetId)(device.as_ptr(), &mut id) } < 0 {
                continue;
            }
            // SAFETY: GetId returns a CoTaskMem string.
            let id = unsafe { take_wide(id) };
            let mut store = null_mut();
            // SAFETY: an IMMDevice and an out pointer.
            let (name, adapter) = if unsafe {
                (device.vtable::<IMMDeviceVtbl>().OpenPropertyStore)(
                    device.as_ptr(),
                    STGM_READ,
                    &mut store,
                )
            } >= 0
                && let Ok(store) = Com::from_raw(store)
            {
                (
                    Self::string_property(&store, &PKEY_DEVICE_FRIENDLY_NAME),
                    Self::string_property(&store, &PKEY_DEVICE_INTERFACE_FRIENDLY_NAME),
                )
            } else {
                (String::new(), String::new())
            };
            let Ok(client) = Self::activate_on(&device, &IID_IAUDIO_CLIENT) else {
                continue;
            };
            let Some(mix) = Self::mix_format(&client) else {
                continue;
            };
            endpoints.push(Endpoint {
                name: if name.is_empty() { id.clone() } else { name },
                id,
                adapter,
                mix,
            });
        }
        Ok(endpoints)
    }

    fn is_format_supported(&self, id: &str, mode: ShareMode, format: &WaveFormat) -> HResult {
        let client = match self
            .device(id)
            .and_then(|device| Self::activate_on(&device, &IID_IAUDIO_CLIENT))
        {
            Ok(client) => client,
            Err(hr) => return hr,
        };
        let wave = wave_format(format);
        let mut closest: *mut c_void = null_mut();
        let (share, out) = match mode {
            ShareMode::Exclusive => (AUDCLNT_SHAREMODE_EXCLUSIVE, null_mut()),
            ShareMode::Shared => (AUDCLNT_SHAREMODE_SHARED, &mut closest as *mut *mut c_void),
        };
        // SAFETY: an IAudioClient, a valid WAVEFORMATEXTENSIBLE.
        let hr = unsafe {
            (client.vtable::<IAudioClientVtbl>().IsFormatSupported)(
                client.as_ptr(),
                share,
                (&wave as *const WAVEFORMATEXTENSIBLE).cast(),
                out,
            )
        };
        if !closest.is_null() {
            // SAFETY: allocated by COM for us to free.
            unsafe { CoTaskMemFree(closest) };
        }
        hr
    }

    fn endpoint_volume(&self, id: &str) -> Result<EndpointVolume, HResult> {
        let device = self.device(id)?;
        let volume = Self::activate_on(&device, &IID_IAUDIO_ENDPOINT_VOLUME)?;
        let table = unsafe { volume.vtable::<IAudioEndpointVolumeVtbl>() };
        let mut support = 0;
        let (mut min_db, mut max_db, mut step_db) = (0.0, 0.0, 0.0);
        // SAFETY: an IAudioEndpointVolume and out values.
        unsafe {
            check((table.QueryHardwareSupport)(volume.as_ptr(), &mut support))?;
            check((table.GetVolumeRange)(
                volume.as_ptr(),
                &mut min_db,
                &mut max_db,
                &mut step_db,
            ))?;
        }
        Ok(EndpointVolume {
            min_db,
            max_db,
            step_db,
            hardware: support & ENDPOINT_HARDWARE_SUPPORT_VOLUME != 0,
        })
    }

    fn set_volume_db(&self, id: &str, db: f32) -> Result<(), HResult> {
        let device = self.device(id)?;
        let volume = Self::activate_on(&device, &IID_IAUDIO_ENDPOINT_VOLUME)?;
        // SAFETY: an IAudioEndpointVolume; no event context.
        check(unsafe {
            (volume
                .vtable::<IAudioEndpointVolumeVtbl>()
                .SetMasterVolumeLevel)(volume.as_ptr(), db, null())
        })
    }

    fn set_mute(&self, id: &str, muted: bool) -> Result<(), HResult> {
        let device = self.device(id)?;
        let volume = Self::activate_on(&device, &IID_IAUDIO_ENDPOINT_VOLUME)?;
        // SAFETY: an IAudioEndpointVolume; no event context.
        check(unsafe {
            (volume.vtable::<IAudioEndpointVolumeVtbl>().SetMute)(
                volume.as_ptr(),
                i32::from(muted),
                null(),
            )
        })
    }

    fn activate(&self, id: &str) -> Result<ComClient, HResult> {
        let device = self.device(id)?;
        Ok(ComClient {
            client: Self::activate_on(&device, &IID_IAUDIO_CLIENT)?,
            render: None,
            clock: None,
            clock_frequency: 0,
            sample_rate: 0,
            block_align: 0,
            event: None,
        })
    }
}

/// The buffer event. Declared after the interfaces in [`ComClient`] so it
/// is closed only once the client has been released.
struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the event this client created.
        unsafe { CloseHandle(self.0) };
    }
}

pub struct ComClient {
    client: Com,
    render: Option<Com>,
    clock: Option<Com>,
    clock_frequency: u64,
    sample_rate: u32,
    /// Bytes per frame of the initialised format.
    block_align: usize,
    event: Option<Event>,
}

// SAFETY: see `Com`; the event handle is a kernel handle usable anywhere.
unsafe impl Send for ComClient {}

impl ComClient {
    fn table(&self) -> &IAudioClientVtbl {
        // SAFETY: `client` is an IAudioClient.
        unsafe { self.client.vtable::<IAudioClientVtbl>() }
    }

    fn service(&self, iid: &GUID) -> Result<Com, HResult> {
        let mut object = null_mut();
        // SAFETY: an initialised IAudioClient and a valid IID.
        check(unsafe { (self.table().GetService)(self.client.as_ptr(), iid, &mut object) })?;
        Com::from_raw(object)
    }
}

impl WasapiClient for ComClient {
    fn device_period(&self) -> Result<(i64, i64), HResult> {
        ensure_com();
        let (mut default, mut minimum) = (0, 0);
        // SAFETY: an IAudioClient and out values.
        check(unsafe {
            (self.table().GetDevicePeriod)(self.client.as_ptr(), &mut default, &mut minimum)
        })?;
        Ok((default, minimum))
    }

    fn initialize(
        &mut self,
        mode: ShareMode,
        event_driven: bool,
        buffer_hns: i64,
        period_hns: i64,
        format: &WaveFormat,
    ) -> Result<(), HResult> {
        ensure_com();
        let wave = wave_format(format);
        let (share, mut flags) = match mode {
            ShareMode::Exclusive => (AUDCLNT_SHAREMODE_EXCLUSIVE, 0),
            ShareMode::Shared => (
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            ),
        };
        if event_driven {
            flags |= AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        }
        // SAFETY: an IAudioClient and a valid WAVEFORMATEXTENSIBLE.
        check(unsafe {
            (self.table().Initialize)(
                self.client.as_ptr(),
                share,
                flags,
                buffer_hns,
                period_hns,
                (&wave as *const WAVEFORMATEXTENSIBLE).cast(),
                null(),
            )
        })?;
        if event_driven {
            // SAFETY: an auto-reset event, unnamed.
            let event = unsafe { CreateEventW(null(), 0, 0, null()) };
            if event.is_null() {
                return Err(super::E_INVALIDARG);
            }
            self.event = Some(Event(event));
            // SAFETY: an initialised client and a valid event.
            check(unsafe { (self.table().SetEventHandle)(self.client.as_ptr(), event) })?;
        }
        self.render = Some(self.service(&IID_IAUDIO_RENDER_CLIENT)?);
        let clock = self.service(&IID_IAUDIO_CLOCK)?;
        let mut frequency = 0;
        // SAFETY: an IAudioClock and an out value.
        check(unsafe {
            (clock.vtable::<IAudioClockVtbl>().GetFrequency)(clock.as_ptr(), &mut frequency)
        })?;
        self.clock_frequency = frequency.max(1);
        self.clock = Some(clock);
        self.sample_rate = format.sample_rate;
        self.block_align = format.block_align();
        Ok(())
    }

    fn buffer_frames(&self) -> Result<u32, HResult> {
        let mut frames = 0;
        // SAFETY: an IAudioClient (valid even after a not-aligned failure).
        check(unsafe { (self.table().GetBufferSize)(self.client.as_ptr(), &mut frames) })?;
        Ok(frames)
    }

    fn padding_frames(&self) -> Result<u32, HResult> {
        let mut frames = 0;
        // SAFETY: an initialised IAudioClient.
        check(unsafe { (self.table().GetCurrentPadding)(self.client.as_ptr(), &mut frames) })?;
        Ok(frames)
    }

    fn wait(&self, timeout_ms: u32) -> bool {
        let Some(event) = &self.event else {
            return false;
        };
        // SAFETY: a valid event handle.
        unsafe { WaitForSingleObject(event.0, timeout_ms) == WAIT_OBJECT_0 }
    }

    fn render(&mut self, frames: u32, data: &[u8]) -> Result<(), HResult> {
        ensure_com();
        let render = self
            .render
            .as_ref()
            .ok_or(super::AUDCLNT_E_NOT_INITIALIZED)?;
        // The buffer GetBuffer hands out holds exactly `frames` frames:
        // refuse data of any other size before asking for it.
        if !super::render_fits(frames, self.block_align, data.len()) {
            return Err(super::E_INVALIDARG);
        }
        // SAFETY: `render` is an IAudioRenderClient (see `Com`).
        let table = unsafe { render.vtable::<IAudioRenderClientVtbl>() };
        let mut buffer: *mut u8 = null_mut();
        // SAFETY: an IAudioRenderClient and an out pointer.
        check(unsafe { (table.GetBuffer)(render.as_ptr(), frames, &mut buffer) })?;
        let Some(buffer) = NonNull::new(buffer) else {
            // Hand the (missing) buffer back so the client stays usable.
            // SAFETY: GetBuffer succeeded, so ReleaseBuffer is due.
            unsafe { (table.ReleaseBuffer)(render.as_ptr(), 0, 0) };
            return Err(E_POINTER);
        };
        // SAFETY: the buffer holds `frames * block_align` bytes, which is
        // `data.len()` (checked above), and stays valid until ReleaseBuffer;
        // nothing touches it after that call.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), buffer.as_ptr(), data.len());
            check((table.ReleaseBuffer)(render.as_ptr(), frames, 0))
        }
    }

    fn start(&mut self) -> Result<(), HResult> {
        ensure_com();
        // SAFETY: an initialised IAudioClient.
        check(unsafe { (self.table().Start)(self.client.as_ptr()) })
    }

    fn stop(&mut self) -> Result<(), HResult> {
        ensure_com();
        // SAFETY: an initialised IAudioClient.
        check(unsafe { (self.table().Stop)(self.client.as_ptr()) })
    }

    fn reset(&mut self) -> Result<(), HResult> {
        ensure_com();
        // SAFETY: an initialised, stopped IAudioClient.
        check(unsafe { (self.table().Reset)(self.client.as_ptr()) })
    }

    fn position_frames(&self) -> Result<u64, HResult> {
        let clock = self
            .clock
            .as_ref()
            .ok_or(super::AUDCLNT_E_NOT_INITIALIZED)?;
        let mut position = 0;
        // SAFETY: an IAudioClock; the QPC out value is optional.
        check(unsafe {
            (clock.vtable::<IAudioClockVtbl>().GetPosition)(
                clock.as_ptr(),
                &mut position,
                null_mut(),
            )
        })?;
        Ok(
            (u128::from(position) * u128::from(self.sample_rate) / u128::from(self.clock_frequency))
                as u64,
        )
    }
}
