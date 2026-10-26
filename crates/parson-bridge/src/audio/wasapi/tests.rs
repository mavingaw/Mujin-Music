use super::fake::{FakeEndpoint, FakeWasapi};
use super::*;

const DAC_GUID: &str = "a1b2c3d4-0000-4000-8000-00000000e30f";
const SPEAKERS_GUID: &str = "5eaa0000-1111-4222-8333-444455556666";

fn dac_endpoint() -> String {
    format!("{{0.0.0.00000000}}.{{{DAC_GUID}}}")
}

fn backend(speed: f64) -> WasapiBackend<FakeWasapi> {
    WasapiBackend::new(FakeWasapi::new(
        vec![
            FakeEndpoint::usb_dac(DAC_GUID, "Topping E30 II"),
            FakeEndpoint::laptop_speakers(SPEAKERS_GUID),
        ],
        speed,
    ))
}

fn noise(frames: usize, bytes_per_sample: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(frames * 2 * bytes_per_sample);
    for _ in 0..frames * 2 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let value = (state >> 8) | 1;
        out.extend_from_slice(&value.to_le_bytes()[..bytes_per_sample]);
    }
    out
}

#[test]
fn lists_every_endpoint_with_its_native_rates_depths_and_hardware_volume() {
    let backend = backend(1.0);
    let devices = backend.devices();
    assert_eq!(devices.len(), 2);
    let dac = &devices[0];
    assert_eq!(dac.id, "wasapi-a1b2c3d400004000800000000000e30f");
    assert_eq!(dac.name, "Speakers (Topping E30 II)");
    assert_eq!(dac.description, "Topping E30 II · WASAPI exclusive");
    assert_eq!(
        dac.sample_rates,
        vec![
            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000
        ]
    );
    assert_eq!(
        dac.formats,
        vec![
            SampleFormat::S16,
            SampleFormat::S24Packed,
            SampleFormat::S32
        ]
    );
    assert_eq!(dac.max_bits(), 32);
    assert_eq!(
        dac.hardware_volume,
        Some(HardwareVolume {
            min_db: -127.5,
            max_db: 0.0,
            step_db: 0.5
        })
    );
    assert!(!dac.busy);
    let speakers = &devices[1];
    assert_eq!(speakers.sample_rates, vec![48_000]);
    assert_eq!(
        speakers.formats,
        vec![SampleFormat::S16, SampleFormat::S24Packed]
    );
    assert!(
        speakers.hardware_volume.is_none(),
        "software endpoint volume is bypassed by exclusive streams, so it is not hardware volume"
    );
    assert_eq!(backend.name(), "WASAPI");
}

#[test]
fn plays_exclusively_at_the_files_own_format_bit_perfect() {
    let backend = backend(8.0);
    let id = backend.devices()[0].id.clone();
    let format = StreamFormat {
        sample_rate: 44_100,
        format: SampleFormat::S24Packed,
        channels: 2,
    };
    let mut stream = backend.open(&id, format, true).expect("exclusive open");
    let data = noise(44_100, 3, 7);
    for chunk in data.chunks(1_000 * 6) {
        stream.write(chunk).expect("write");
    }
    assert!(
        stream.delay_frames() > 0,
        "audio is buffered while it plays"
    );
    stream.drain();
    assert_eq!(stream.delay_frames(), 0);
    drop(stream);
    let capture = backend.api().capture(&dac_endpoint());
    // 441 frames of 6 bytes are not 128-byte aligned: the documented retry
    // with a fresh client and the aligned period.
    assert_eq!(capture.alignment_retries, 1);
    assert_eq!(capture.activations, 2);
    assert_eq!(
        capture.opened,
        vec![(
            WaveFormat {
                sample_rate: 44_100,
                channels: 2,
                container_bits: 24,
                valid_bits: 24
            },
            ShareMode::Exclusive,
            448
        )]
    );
    assert!(
        capture.bytes.len() >= data.len() && capture.bytes[..data.len()] == data[..],
        "every sample reached the DAC unchanged"
    );
    assert!(
        capture.bytes[data.len()..].iter().all(|byte| *byte == 0),
        "only silence follows the music while it drains"
    );
}

#[test]
fn opens_32_bit_containers_with_the_valid_bits_the_dac_takes() {
    let backend = backend(16.0);
    let id = backend.devices()[0].id.clone();
    let format = StreamFormat {
        sample_rate: 192_000,
        format: SampleFormat::S32,
        channels: 2,
    };
    let mut stream = backend.open(&id, format, true).expect("open");
    let data = noise(19_200, 4, 3);
    stream.write(&data).expect("write");
    stream.drain();
    drop(stream);
    let capture = backend.api().capture(&dac_endpoint());
    let (wave, mode, _) = capture.opened[0];
    assert_eq!(mode, ShareMode::Exclusive);
    assert_eq!((wave.container_bits, wave.valid_bits), (32, 24));
    assert!(capture.bytes[..data.len()] == data[..]);
}

#[test]
fn explains_refusals_instead_of_converting() {
    let backend = backend(1.0);
    let devices = backend.devices();
    let speakers = devices[1].id.clone();
    let error = backend
        .open(
            &speakers,
            StreamFormat {
                sample_rate: 96_000,
                format: SampleFormat::S24Packed,
                channels: 2,
            },
            true,
        )
        .err()
        .expect("refused");
    assert_eq!(
        error,
        "Speakers (Realtek(R) Audio) does not take 96000 Hz S24_3LE in exclusive mode"
    );

    // Another app takes the DAC exclusively: listed as busy, opens fail.
    let dac = devices[0].id.clone();
    backend.api().update(&dac_endpoint(), |endpoint| {
        endpoint.held_by_other_app = true
    });
    let listed = backend.devices();
    assert!(listed[0].busy, "{listed:?}");
    assert_eq!(listed[0].sample_rates.len(), 8, "keeps what it took before");
    let error = backend
        .open(
            &dac,
            StreamFormat {
                sample_rate: 44_100,
                format: SampleFormat::S16,
                channels: 2,
            },
            true,
        )
        .err()
        .expect("in use");
    assert!(error.contains("in use by another app"), "{error}");
}

#[test]
fn one_stream_holds_the_device_and_lets_go_when_dropped() {
    let backend = backend(8.0);
    let id = backend.devices()[0].id.clone();
    let format = StreamFormat {
        sample_rate: 48_000,
        format: SampleFormat::S16,
        channels: 2,
    };
    let stream = backend.open(&id, format, true).expect("first");
    let listed = backend.devices();
    assert!(!listed[0].busy, "Parson's own stream is not another app");
    assert!(
        backend.open(&id, format, true).is_err(),
        "one exclusive client"
    );
    drop(stream);
    assert!(backend.open(&id, format, true).is_ok(), "free again");
}

#[test]
fn pauses_in_place_and_drops_its_buffer_on_seek() {
    let backend = backend(1.0);
    let id = backend.devices()[0].id.clone();
    let format = StreamFormat {
        sample_rate: 48_000,
        format: SampleFormat::S16,
        channels: 2,
    };
    let mut stream = backend.open(&id, format, true).expect("open");
    stream.write(&noise(4_800, 2, 1)).expect("write");
    stream.pause(true).expect("pause");
    let held = stream.delay_frames();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(stream.delay_frames(), held, "nothing plays while paused");
    stream.pause(false).expect("resume");
    stream.drop_buffer();
    assert_eq!(stream.delay_frames(), 0, "a seek starts from empty");
    let data = noise(960, 2, 9);
    stream.write(&data).expect("write after seek");
    stream.drain();
    drop(stream);
    let capture = backend.api().capture(&dac_endpoint());
    let found = capture
        .bytes
        .windows(data.len())
        .any(|window| window == data.as_slice());
    assert!(found, "the audio after the seek played whole");
}

#[test]
fn hardware_volume_is_set_in_db_on_the_dac() {
    let backend = backend(1.0);
    let devices = backend.devices();
    backend
        .set_hardware_volume(&devices[0].id, -23.5)
        .expect("volume");
    backend
        .set_hardware_volume(&devices[0].id, -300.0)
        .expect("clamped");
    backend
        .set_hardware_mute(&devices[0].id, true)
        .expect("mute");
    let capture = backend.api().capture(&dac_endpoint());
    assert_eq!(capture.volume_db, Some(-127.5));
    assert!(capture.muted);
    assert!(
        backend.set_hardware_volume(&devices[1].id, -10.0).is_err(),
        "software endpoint volume is not offered"
    );
}

#[test]
fn a_device_with_exclusive_mode_turned_off_plays_shared_and_says_so() {
    let backend = backend(8.0);
    backend.api().update(&dac_endpoint(), |endpoint| {
        endpoint.exclusive_allowed = false;
    });
    let devices = backend.devices();
    let dac = &devices[0];
    assert_eq!(dac.sample_rates, vec![48_000]);
    assert!(dac.description.contains("not bit-perfect"), "{dac:?}");
    let format = StreamFormat {
        sample_rate: 48_000,
        format: SampleFormat::S16,
        channels: 2,
    };
    let mut stream = backend.open(&dac.id, format, true).expect("shared open");
    let data = noise(9_600, 2, 4);
    stream.write(&data).expect("write");
    stream.drain();
    drop(stream);
    let capture = backend.api().capture(&dac_endpoint());
    assert_eq!(capture.opened[0].1, ShareMode::Shared);
    assert!(capture.bytes.starts_with(&data));
}

#[test]
fn unplugged_endpoints_disappear() {
    let backend = backend(1.0);
    let id = backend.devices()[0].id.clone();
    backend.api().unplug(&dac_endpoint());
    assert_eq!(backend.devices().len(), 1);
    let format = StreamFormat {
        sample_rate: 44_100,
        format: SampleFormat::S16,
        channels: 2,
    };
    assert!(backend.open(&id, format, true).is_err());
}
