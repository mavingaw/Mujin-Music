use super::fake::{FakeHal, FakeHalDevice, OTHER_PROCESS, THIS_PROCESS};
use super::*;

const DAC: u32 = 73;
const SPEAKERS: u32 = 41;

fn backend(speed: f64) -> CoreAudioBackend<FakeHal> {
    CoreAudioBackend::new(FakeHal::new(
        vec![
            FakeHalDevice::usb_dac(DAC, "AppleUSBAudioEngine:Chord:Qutest:1400000:1", "Qutest"),
            FakeHalDevice::built_in(SPEAKERS),
        ],
        speed,
    ))
}

/// Deterministic integer samples, `bits` deep, as little-endian bytes in
/// `bytes`-byte containers (left-aligned in 32-bit containers).
fn noise(frames: usize, bits: u32, bytes: usize, seed: u32) -> (Vec<u8>, Vec<i32>) {
    let mut state = seed;
    let mut raw = Vec::new();
    let mut values = Vec::new();
    for _ in 0..frames * 2 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let value = ((state as i32) >> (32 - bits)) | 1;
        values.push(value);
        let stored = if bytes == 4 { value << 8 } else { value };
        raw.extend_from_slice(&stored.to_le_bytes()[..bytes]);
    }
    (raw, values)
}

/// The capture from the first sample that is not silence.
fn audio(samples: &[i32]) -> &[i32] {
    let first = samples
        .chunks(2)
        .position(|frame| frame.iter().any(|sample| *sample != 0))
        .unwrap_or(0);
    &samples[first * 2..]
}

#[test]
fn lists_outputs_with_their_rates_depths_and_hardware_volume() {
    let backend = backend(1.0);
    let devices = backend.devices();
    assert_eq!(devices.len(), 2);
    let dac = &devices[0];
    assert_eq!(
        dac.id,
        "coreaudio-appleusbaudioengine-chord-qutest-1400000-1"
    );
    assert_eq!(dac.name, "Qutest");
    assert_eq!(
        dac.description,
        "Chord Electronics Ltd · USB · CoreAudio hog mode"
    );
    assert_eq!(
        dac.sample_rates,
        vec![
            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000
        ]
    );
    assert_eq!(
        dac.formats,
        vec![SampleFormat::S16, SampleFormat::S24Packed],
        "the HAL's float stage carries 24 bits exactly, so 32-bit sources are not offered"
    );
    assert_eq!(
        dac.hardware_volume,
        Some(HardwareVolume {
            min_db: -96.0,
            max_db: 0.0,
            step_db: 0.5
        })
    );
    let speakers = &devices[1];
    assert_eq!(
        speakers.sample_rates,
        vec![44_100, 48_000, 88_200, 96_000],
        "a continuous range lists the standard rates inside it"
    );
    assert_eq!(backend.name(), "CoreAudio");
}

#[test]
fn hogs_the_dac_switches_its_rate_and_plays_24_bit_bit_perfect() {
    let backend = backend(8.0);
    let id = backend.devices()[0].id.clone();
    // The DAC sits at 44.1 kHz; the file is 96 kHz/24-bit.
    let format = StreamFormat {
        sample_rate: 96_000,
        format: SampleFormat::S24Packed,
        channels: 2,
    };
    let mut stream = backend.open(&id, format, true).expect("open");
    let hal = backend.api();
    assert_eq!(hal.hog_owner(DAC), Ok(THIS_PROCESS), "hog mode taken");
    assert_eq!(hal.nominal(DAC), 96_000.0, "waited for the switch to land");
    let physical = hal.physical(DAC).expect("physical format");
    assert_eq!((physical.bits, physical.float), (24, false));
    let (raw, values) = noise(48_000, 24, 3, 5);
    for chunk in raw.chunks(4_800 * 6) {
        stream.write(chunk).expect("write");
    }
    stream.drain();
    drop(stream);
    assert_eq!(hal.hog_owner(DAC), Ok(-1), "released when done");
    let capture = hal.capture(DAC);
    assert_eq!(capture.rate_requests, vec![96_000.0]);
    assert_eq!(capture.runs, vec![(24, 96_000.0)]);
    let played = audio(&capture.samples);
    assert!(
        played.len() >= values.len() && played[..values.len()] == values[..],
        "every 24-bit sample reached the DAC unchanged"
    );
}

#[test]
fn sixteen_bit_files_run_the_dac_at_16_bits_and_32_bit_sources_are_refused() {
    let backend = backend(8.0);
    let id = backend.devices()[0].id.clone();
    let mut stream = backend
        .open(
            &id,
            StreamFormat {
                sample_rate: 44_100,
                format: SampleFormat::S16,
                channels: 2,
            },
            true,
        )
        .expect("open 16-bit");
    let (raw, values) = noise(22_050, 16, 2, 9);
    stream.write(&raw).expect("write");
    stream.drain();
    drop(stream);
    let hal = backend.api();
    assert_eq!(hal.physical(DAC).map(|format| format.bits), Some(16));
    let played = audio(&hal.capture(DAC).samples).to_vec();
    assert!(played[..values.len()] == values[..], "16-bit bit-perfect");

    // 32-bit containers are not listed (the float stage keeps 24 bits), so
    // they are refused rather than silently truncated.
    let error = backend
        .open(
            &id,
            StreamFormat {
                sample_rate: 44_100,
                format: SampleFormat::S32,
                channels: 2,
            },
            true,
        )
        .err()
        .expect("refused");
    assert!(error.contains("S32_LE"), "{error}");
}

#[test]
fn another_app_in_hog_mode_makes_the_dac_busy() {
    let backend = backend(1.0);
    let id = backend.devices()[0].id.clone();
    backend.api().set_hog(DAC, OTHER_PROCESS);
    let devices = backend.devices();
    assert!(devices[0].busy);
    let error = backend
        .open(
            &id,
            StreamFormat {
                sample_rate: 44_100,
                format: SampleFormat::S16,
                channels: 2,
            },
            true,
        )
        .err()
        .expect("refused");
    assert!(
        error.contains("another app has the device in hog mode"),
        "{error}"
    );
    assert_eq!(
        backend.api().hog_owner(DAC),
        Ok(OTHER_PROCESS),
        "the toggle never takes it from them"
    );
}

#[test]
fn pause_plays_silence_without_letting_go_and_seek_drops_the_buffer() {
    let backend = backend(1.0);
    let id = backend.devices()[0].id.clone();
    let format = StreamFormat {
        sample_rate: 48_000,
        format: SampleFormat::S16,
        channels: 2,
    };
    let mut stream = backend.open(&id, format, true).expect("open");
    let (raw, _) = noise(4_800, 16, 2, 2);
    stream.write(&raw).expect("write");
    stream.pause(true).expect("pause");
    let held = stream.delay_frames();
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(stream.delay_frames(), held, "nothing plays while paused");
    assert_eq!(backend.api().hog_owner(DAC), Ok(THIS_PROCESS));
    stream.drop_buffer();
    stream.pause(false).expect("resume");
    assert!(stream.delay_frames() <= u64::from(backend.api().latency_frames(DAC)));
    drop(stream);
}

#[test]
fn shared_mode_leaves_hog_mode_alone_and_volume_is_in_db() {
    let backend = backend(8.0);
    let devices = backend.devices();
    let stream = backend
        .open(
            &devices[1].id,
            StreamFormat {
                sample_rate: 48_000,
                format: SampleFormat::S16,
                channels: 2,
            },
            false,
        )
        .expect("shared open");
    assert_eq!(backend.api().hog_owner(SPEAKERS), Ok(-1));
    assert_eq!(backend.api().nominal(SPEAKERS), 48_000.0);
    drop(stream);
    backend
        .set_hardware_volume(&devices[0].id, -12.5)
        .expect("volume");
    backend
        .set_hardware_volume(&devices[0].id, -200.0)
        .expect("clamped");
    backend
        .set_hardware_mute(&devices[0].id, true)
        .expect("mute");
    let capture = backend.api().capture(DAC);
    assert_eq!(capture.volume_db, Some(-96.0));
    assert!(capture.muted);
}

#[test]
fn refuses_rates_the_device_does_not_have() {
    let backend = backend(1.0);
    let devices = backend.devices();
    let error = backend
        .open(
            &devices[1].id,
            StreamFormat {
                sample_rate: 192_000,
                format: SampleFormat::S16,
                channels: 2,
            },
            true,
        )
        .err()
        .expect("refused");
    assert_eq!(error, "MacBook Pro Speakers does not take 192000 Hz");
    assert_eq!(
        backend.api().hog_owner(SPEAKERS),
        Ok(-1),
        "a refused open does not keep hog mode"
    );
}

#[test]
fn describes_hal_errors_readably() {
    assert_eq!(
        describe(PERMISSIONS),
        "another app has the device in hog mode ('!hog')"
    );
    assert_eq!(describe(-50), "CoreAudio error -50");
}
