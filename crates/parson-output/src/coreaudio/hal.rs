//! The real macOS audio HAL: `AudioObjectGetPropertyData` /
//! `AudioObjectSetPropertyData` for devices, rates, physical formats, hog
//! mode and volume, and an `AudioDeviceIOProc` for playback.

#![allow(non_upper_case_globals, non_snake_case)]

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::Mutex;

use super::{
    CoreAudioBackend, HalApi, HalDevice, NO_ERR, OsStatus, PhysicalFormat, RenderCallback,
    UNKNOWN_PROPERTY,
};

type AudioObjectID = u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioObjectPropertyAddress {
    mSelector: u32,
    mScope: u32,
    mElement: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioValueRange {
    mMinimum: f64,
    mMaximum: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioStreamBasicDescription {
    mSampleRate: f64,
    mFormatID: u32,
    mFormatFlags: u32,
    mBytesPerPacket: u32,
    mFramesPerPacket: u32,
    mBytesPerFrame: u32,
    mChannelsPerFrame: u32,
    mBitsPerChannel: u32,
    mReserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioStreamRangedDescription {
    mFormat: AudioStreamBasicDescription,
    mSampleRateRange: AudioValueRange,
}

#[repr(C)]
struct AudioBuffer {
    mNumberChannels: u32,
    mDataByteSize: u32,
    mData: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    mNumberBuffers: u32,
    mBuffers: [AudioBuffer; 1],
}

type AudioDeviceIOProcID = *mut c_void;
type AudioDeviceIOProc = unsafe extern "C" fn(
    AudioObjectID,
    *const c_void,
    *const AudioBufferList,
    *const c_void,
    *mut AudioBufferList,
    *const c_void,
    *mut c_void,
) -> OsStatus;

const fn code(value: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*value)
}

const kAudioObjectSystemObject: AudioObjectID = 1;
const kAudioObjectPropertyElementMain: u32 = 0;
const kAudioObjectPropertyScopeGlobal: u32 = code(b"glob");
const kAudioObjectPropertyScopeOutput: u32 = code(b"outp");
const kAudioHardwarePropertyDevices: u32 = code(b"dev#");
const kAudioObjectPropertyName: u32 = code(b"lnam");
const kAudioObjectPropertyManufacturer: u32 = code(b"lmak");
const kAudioDevicePropertyDeviceUID: u32 = code(b"uid ");
const kAudioDevicePropertyTransportType: u32 = code(b"tran");
const kAudioDevicePropertyStreamConfiguration: u32 = code(b"slay");
const kAudioDevicePropertyAvailableNominalSampleRates: u32 = code(b"nsr#");
const kAudioDevicePropertyNominalSampleRate: u32 = code(b"nsrt");
const kAudioDevicePropertyStreams: u32 = code(b"stm#");
const kAudioStreamPropertyAvailablePhysicalFormats: u32 = code(b"pft#");
const kAudioStreamPropertyPhysicalFormat: u32 = code(b"pft ");
const kAudioDevicePropertyHogMode: u32 = code(b"oink");
const kAudioDevicePropertyVolumeRangeDecibels: u32 = code(b"vdb#");
const kAudioDevicePropertyVolumeDecibels: u32 = code(b"vcdb");
const kAudioDevicePropertyMute: u32 = code(b"mute");
const kAudioDevicePropertyLatency: u32 = code(b"ltnc");
const kAudioDevicePropertySafetyOffset: u32 = code(b"saft");
const kAudioDevicePropertyBufferFrameSize: u32 = code(b"fsiz");
const kAudioFormatLinearPCM: u32 = code(b"lpcm");
const kAudioFormatFlagIsFloat: u32 = 1 << 0;
const kAudioFormatFlagIsSignedInteger: u32 = 1 << 2;
const kAudioFormatFlagIsPacked: u32 = 1 << 3;
const kCFStringEncodingUTF8: u32 = 0x0800_0100;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectHasProperty(id: AudioObjectID, address: *const AudioObjectPropertyAddress) -> u8;
    fn AudioObjectGetPropertyDataSize(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> OsStatus;
    fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectSetPropertyData(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OsStatus;
    fn AudioDeviceCreateIOProcID(
        device: AudioObjectID,
        proc_: AudioDeviceIOProc,
        client: *mut c_void,
        id: *mut AudioDeviceIOProcID,
    ) -> OsStatus;
    fn AudioDeviceDestroyIOProcID(device: AudioObjectID, id: AudioDeviceIOProcID) -> OsStatus;
    fn AudioDeviceStart(device: AudioObjectID, id: AudioDeviceIOProcID) -> OsStatus;
    fn AudioDeviceStop(device: AudioObjectID, id: AudioDeviceIOProcID) -> OsStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetLength(string: *const c_void) -> isize;
    fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> u8;
    fn CFRelease(object: *const c_void);
}

fn property(selector: u32, scope: u32, element: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: element,
    }
}

fn has(id: AudioObjectID, address: &AudioObjectPropertyAddress) -> bool {
    // SAFETY: a valid address struct.
    unsafe { AudioObjectHasProperty(id, address) != 0 }
}

/// Reads a fixed-size property.
fn get<T: Copy + Default>(
    id: AudioObjectID,
    address: &AudioObjectPropertyAddress,
) -> Result<T, OsStatus> {
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: `value` has room for `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            address,
            0,
            std::ptr::null(),
            &mut size,
            (&mut value as *mut T).cast(),
        )
    };
    if status == NO_ERR {
        Ok(value)
    } else {
        Err(status)
    }
}

fn set<T: Copy>(
    id: AudioObjectID,
    address: &AudioObjectPropertyAddress,
    value: &T,
) -> Result<(), OsStatus> {
    // SAFETY: `value` is `size_of::<T>()` readable bytes.
    let status = unsafe {
        AudioObjectSetPropertyData(
            id,
            address,
            0,
            std::ptr::null(),
            std::mem::size_of::<T>() as u32,
            (value as *const T).cast(),
        )
    };
    if status == NO_ERR {
        Ok(())
    } else {
        Err(status)
    }
}

/// Reads an array property.
fn get_array<T: Copy + Default>(
    id: AudioObjectID,
    address: &AudioObjectPropertyAddress,
) -> Result<Vec<T>, OsStatus> {
    let mut size = 0;
    // SAFETY: a valid address and out size.
    let status =
        unsafe { AudioObjectGetPropertyDataSize(id, address, 0, std::ptr::null(), &mut size) };
    if status != NO_ERR {
        return Err(status);
    }
    let count = size as usize / std::mem::size_of::<T>().max(1);
    let mut values = vec![T::default(); count];
    let mut size = (count * std::mem::size_of::<T>()) as u32;
    // SAFETY: `values` has room for `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            address,
            0,
            std::ptr::null(),
            &mut size,
            values.as_mut_ptr().cast(),
        )
    };
    if status != NO_ERR {
        return Err(status);
    }
    values.truncate(size as usize / std::mem::size_of::<T>().max(1));
    Ok(values)
}

fn string(id: AudioObjectID, selector: u32) -> String {
    let address = property(
        selector,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    );
    let Ok(pointer) = get::<usize>(id, &address) else {
        return String::new();
    };
    if pointer == 0 {
        return String::new();
    }
    let reference = pointer as *const c_void;
    // SAFETY: a CFStringRef we own (the HAL returns a retained string).
    unsafe {
        let length = CFStringGetLength(reference);
        let capacity = length * 4 + 1;
        let mut buffer = vec![0 as c_char; capacity.max(1) as usize];
        let ok = CFStringGetCString(
            reference,
            buffer.as_mut_ptr(),
            capacity,
            kCFStringEncodingUTF8,
        );
        CFRelease(reference);
        if ok == 0 {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        }
    }
}

fn output_channels(id: AudioObjectID) -> u32 {
    let address = property(
        kAudioDevicePropertyStreamConfiguration,
        kAudioObjectPropertyScopeOutput,
        kAudioObjectPropertyElementMain,
    );
    // The AudioBufferList is variable length: read it as raw words.
    let Ok(words) = get_array::<u64>(id, &address) else {
        return 0;
    };
    if words.is_empty() {
        return 0;
    }
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_ne_bytes()).collect();
    let buffers = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let stride = std::mem::size_of::<AudioBuffer>();
    let offset = std::mem::offset_of!(AudioBufferList, mBuffers);
    (0..buffers)
        .filter_map(|index| {
            let start = offset + index * stride;
            bytes.get(start..start + 4).map(|channels| {
                u32::from_ne_bytes([channels[0], channels[1], channels[2], channels[3]])
            })
        })
        .sum()
}

fn first_output_stream(device: AudioObjectID) -> Result<AudioObjectID, OsStatus> {
    let address = property(
        kAudioDevicePropertyStreams,
        kAudioObjectPropertyScopeOutput,
        kAudioObjectPropertyElementMain,
    );
    get_array::<u32>(device, &address)?
        .first()
        .copied()
        .ok_or(UNKNOWN_PROPERTY)
}

/// The volume control: the main element, or the first channel's.
fn volume_element(device: AudioObjectID, selector: u32) -> Option<AudioObjectPropertyAddress> {
    [kAudioObjectPropertyElementMain, 1]
        .into_iter()
        .map(|element| property(selector, kAudioObjectPropertyScopeOutput, element))
        .find(|address| has(device, address))
}

struct IoState {
    render: RenderCallback,
    scratch: Vec<f32>,
}

/// The IOProc: Parson fills one interleaved buffer, which is then spread
/// over the device's output buffers (one interleaved buffer on most DACs).
unsafe extern "C" fn io_proc(
    _device: AudioObjectID,
    _now: *const c_void,
    _input: *const AudioBufferList,
    _input_time: *const c_void,
    output: *mut AudioBufferList,
    _output_time: *const c_void,
    client: *mut c_void,
) -> OsStatus {
    if output.is_null() || client.is_null() {
        return NO_ERR;
    }
    // SAFETY: `client` is the IoState registered with this IOProc, and
    // the HAL calls it from one thread at a time.
    let state = unsafe { &mut *(client as *mut IoState) };
    // SAFETY: the HAL passes a valid list of `mNumberBuffers` buffers.
    let buffers = unsafe {
        std::slice::from_raw_parts_mut(
            (*output).mBuffers.as_mut_ptr(),
            (*output).mNumberBuffers as usize,
        )
    };
    let total: usize = buffers
        .iter()
        .map(|buffer| buffer.mNumberChannels as usize)
        .sum();
    let Some(first) = buffers.first() else {
        return NO_ERR;
    };
    if total == 0 || first.mNumberChannels == 0 {
        return NO_ERR;
    }
    let frames = first.mDataByteSize as usize / (4 * first.mNumberChannels as usize);
    let needed = frames * total;
    if state.scratch.len() < needed {
        // Only on the first cycles: the capacity is reserved up front.
        state.scratch.resize(needed, 0.0);
    }
    let scratch = &mut state.scratch[..needed];
    (state.render)(scratch, total);
    let mut channel_offset = 0;
    for buffer in buffers.iter_mut() {
        let channels = buffer.mNumberChannels as usize;
        if buffer.mData.is_null() || channels == 0 {
            channel_offset += channels;
            continue;
        }
        let samples = (buffer.mDataByteSize as usize / 4).min(frames * channels);
        // SAFETY: the HAL's float output buffer of mDataByteSize bytes.
        let out = unsafe { std::slice::from_raw_parts_mut(buffer.mData as *mut f32, samples) };
        for frame in 0..samples / channels {
            for channel in 0..channels {
                out[frame * channels + channel] = scratch[frame * total + channel_offset + channel];
            }
        }
        channel_offset += channels;
    }
    NO_ERR
}

struct Running {
    proc_id: usize,
    state: usize,
}

pub struct MacHal {
    running: Mutex<HashMap<u64, Running>>,
    next: Mutex<u64>,
}

/// The machine's CoreAudio.
pub fn load() -> Result<CoreAudioBackend<MacHal>, String> {
    let address = property(
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    );
    get_array::<u32>(kAudioObjectSystemObject, &address)
        .map_err(|status| format!("CoreAudio is unavailable: {}", super::describe(status)))?;
    Ok(CoreAudioBackend::new(MacHal {
        running: Mutex::new(HashMap::new()),
        next: Mutex::new(1),
    }))
}

impl HalApi for MacHal {
    fn output_devices(&self) -> Result<Vec<HalDevice>, OsStatus> {
        let address = property(
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain,
        );
        let ids = get_array::<u32>(kAudioObjectSystemObject, &address)?;
        Ok(ids
            .into_iter()
            .map(|id| HalDevice {
                object_id: id,
                uid: string(id, kAudioDevicePropertyDeviceUID),
                name: string(id, kAudioObjectPropertyName),
                manufacturer: string(id, kAudioObjectPropertyManufacturer),
                transport: get::<u32>(
                    id,
                    &property(
                        kAudioDevicePropertyTransportType,
                        kAudioObjectPropertyScopeGlobal,
                        kAudioObjectPropertyElementMain,
                    ),
                )
                .unwrap_or(0),
                output_channels: output_channels(id),
            })
            .filter(|device| device.output_channels > 0 && !device.uid.is_empty())
            .collect())
    }

    fn available_rates(&self, device: u32) -> Result<Vec<(f64, f64)>, OsStatus> {
        let address = property(
            kAudioDevicePropertyAvailableNominalSampleRates,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain,
        );
        Ok(get_array::<AudioValueRange>(device, &address)?
            .into_iter()
            .map(|range| (range.mMinimum, range.mMaximum))
            .collect())
    }

    fn nominal_rate(&self, device: u32) -> Result<f64, OsStatus> {
        get::<f64>(
            device,
            &property(
                kAudioDevicePropertyNominalSampleRate,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
        )
    }

    fn set_nominal_rate(&self, device: u32, rate: f64) -> Result<(), OsStatus> {
        set(
            device,
            &property(
                kAudioDevicePropertyNominalSampleRate,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
            &rate,
        )
    }

    fn physical_formats(&self, device: u32) -> Result<Vec<PhysicalFormat>, OsStatus> {
        let stream = first_output_stream(device)?;
        let address = property(
            kAudioStreamPropertyAvailablePhysicalFormats,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain,
        );
        Ok(get_array::<AudioStreamRangedDescription>(stream, &address)?
            .into_iter()
            .filter(|ranged| ranged.mFormat.mFormatID == kAudioFormatLinearPCM)
            .map(|ranged| PhysicalFormat {
                rate_min: ranged.mSampleRateRange.mMinimum,
                rate_max: ranged.mSampleRateRange.mMaximum,
                bits: ranged.mFormat.mBitsPerChannel,
                float: ranged.mFormat.mFormatFlags & kAudioFormatFlagIsFloat != 0,
                channels: ranged.mFormat.mChannelsPerFrame,
            })
            .collect())
    }

    fn set_physical_format(&self, device: u32, format: &PhysicalFormat) -> Result<(), OsStatus> {
        let stream = first_output_stream(device)?;
        // Match the listed description exactly (container size, packing).
        let listed = get_array::<AudioStreamRangedDescription>(
            stream,
            &property(
                kAudioStreamPropertyAvailablePhysicalFormats,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
        )?;
        let mut description = listed
            .iter()
            .map(|ranged| ranged.mFormat)
            .find(|candidate| {
                candidate.mFormatID == kAudioFormatLinearPCM
                    && candidate.mBitsPerChannel == format.bits
                    && candidate.mChannelsPerFrame == format.channels
                    && (candidate.mFormatFlags & kAudioFormatFlagIsFloat != 0) == format.float
            })
            .unwrap_or(AudioStreamBasicDescription {
                mFormatID: kAudioFormatLinearPCM,
                mFormatFlags: if format.float {
                    kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked
                } else {
                    kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked
                },
                mBytesPerPacket: format.bits / 8 * format.channels,
                mFramesPerPacket: 1,
                mBytesPerFrame: format.bits / 8 * format.channels,
                mChannelsPerFrame: format.channels,
                mBitsPerChannel: format.bits,
                ..AudioStreamBasicDescription::default()
            });
        description.mSampleRate = format.rate_min;
        set(
            stream,
            &property(
                kAudioStreamPropertyPhysicalFormat,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
            &description,
        )
    }

    fn hog_owner(&self, device: u32) -> Result<i32, OsStatus> {
        get::<i32>(
            device,
            &property(
                kAudioDevicePropertyHogMode,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
        )
    }

    fn toggle_hog(&self, device: u32) -> Result<i32, OsStatus> {
        let address = property(
            kAudioDevicePropertyHogMode,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain,
        );
        set(device, &address, &self.process_id())?;
        get::<i32>(device, &address)
    }

    fn volume_range_db(&self, device: u32) -> Option<(f32, f32)> {
        let address = volume_element(device, kAudioDevicePropertyVolumeRangeDecibels)?;
        let range = get::<AudioValueRange>(device, &address).ok()?;
        Some((range.mMinimum as f32, range.mMaximum as f32))
    }

    fn set_volume_db(&self, device: u32, db: f32) -> Result<(), OsStatus> {
        let main = property(
            kAudioDevicePropertyVolumeDecibels,
            kAudioObjectPropertyScopeOutput,
            kAudioObjectPropertyElementMain,
        );
        if has(device, &main) {
            return set(device, &main, &db);
        }
        // Per-channel controls only: set each.
        let mut any = false;
        for element in 1..=output_channels(device).max(1) {
            let channel = property(
                kAudioDevicePropertyVolumeDecibels,
                kAudioObjectPropertyScopeOutput,
                element,
            );
            if has(device, &channel) {
                set(device, &channel, &db)?;
                any = true;
            }
        }
        if any { Ok(()) } else { Err(UNKNOWN_PROPERTY) }
    }

    fn set_mute(&self, device: u32, muted: bool) -> Result<(), OsStatus> {
        let address = volume_element(device, kAudioDevicePropertyMute).ok_or(UNKNOWN_PROPERTY)?;
        set(device, &address, &u32::from(muted))
    }

    fn latency_frames(&self, device: u32) -> u32 {
        let output = |selector| {
            get::<u32>(
                device,
                &property(
                    selector,
                    kAudioObjectPropertyScopeOutput,
                    kAudioObjectPropertyElementMain,
                ),
            )
            .unwrap_or(0)
        };
        let buffer = get::<u32>(
            device,
            &property(
                kAudioDevicePropertyBufferFrameSize,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            ),
        )
        .unwrap_or(0);
        output(kAudioDevicePropertyLatency) + output(kAudioDevicePropertySafetyOffset) + buffer
    }

    fn start(&self, device: u32, render: RenderCallback) -> Result<u64, OsStatus> {
        let state = Box::into_raw(Box::new(IoState {
            render,
            scratch: Vec::with_capacity(16_384 * 8),
        }));
        let mut proc_id: AudioDeviceIOProcID = std::ptr::null_mut();
        // SAFETY: `io_proc` matches AudioDeviceIOProc; `state` stays alive
        // until the IOProc is destroyed in `stop`.
        let status =
            unsafe { AudioDeviceCreateIOProcID(device, io_proc, state.cast(), &mut proc_id) };
        if status != NO_ERR {
            // SAFETY: never registered, so ours to free.
            drop(unsafe { Box::from_raw(state) });
            return Err(status);
        }
        // SAFETY: the IOProc just created.
        let status = unsafe { AudioDeviceStart(device, proc_id) };
        if status != NO_ERR {
            // SAFETY: as above; the IOProc never ran.
            unsafe {
                AudioDeviceDestroyIOProcID(device, proc_id);
                drop(Box::from_raw(state));
            }
            return Err(status);
        }
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let handle = *next;
        *next += 1;
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                handle,
                Running {
                    proc_id: proc_id as usize,
                    state: state as usize,
                },
            );
        Ok(handle)
    }

    fn stop(&self, device: u32, handle: u64) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&handle);
        if let Some(running) = running {
            let proc_id = running.proc_id as AudioDeviceIOProcID;
            // SAFETY: AudioDeviceStop waits for the IOProc to return, so
            // once it is destroyed nothing touches `state` any more.
            unsafe {
                AudioDeviceStop(device, proc_id);
                AudioDeviceDestroyIOProcID(device, proc_id);
                drop(Box::from_raw(running.state as *mut IoState));
            }
        }
    }

    fn process_id(&self) -> i32 {
        // SAFETY: plain libc call.
        unsafe { libc::getpid() }
    }
}
