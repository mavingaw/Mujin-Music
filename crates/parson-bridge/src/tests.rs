//! End-to-end tests over real TCP sockets: a bridge endpoint with simulated
//! DACs (which consume audio in real time like hardware, enforce their
//! formats and keep every byte they were sent) and the server-side client.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::endpoint::{Endpoint, EndpointConfig};
use crate::remote::{GroupMember, RemoteBridge, SyncGroup, common_device};
use parson_output::simulated::SimulatedBackend;
use parson_output::{Backend, SampleFormat, Sink, StreamFormat};

fn endpoint(name: &str, speed: f64, clock_offset_us: i64) -> (Endpoint, Arc<SimulatedBackend>) {
    let backend = Arc::new(SimulatedBackend::with_default_devices(speed));
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let endpoint = Endpoint::start(
        listener,
        Some(backend.clone() as Arc<dyn Backend>),
        EndpointConfig {
            bridge_id: format!("{name}-id"),
            name: name.to_string(),
            host: format!("{name}.local"),
            clock_offset_us,
            note: None,
        },
    )
    .expect("endpoint");
    (endpoint, backend)
}

fn connect(endpoint: &Endpoint) -> RemoteBridge {
    RemoteBridge::connect(
        endpoint.address(),
        "server-1",
        "Test server",
        Duration::from_secs(5),
    )
    .expect("connect")
}

/// Deterministic noise with no all-zero frames, `bytes` per sample.
fn noise(frames: usize, bytes: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(frames * 2 * bytes);
    for _ in 0..frames * 2 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let value = (state >> 8) | 1;
        out.extend_from_slice(&value.to_le_bytes()[..bytes]);
    }
    out
}

#[test]
fn a_remote_dac_gets_every_sample_bit_perfect() {
    let (endpoint, backend) = endpoint("pi", 8.0, 0);
    let bridge = connect(&endpoint);
    let info = bridge.info();
    assert_eq!(
        (info.name.as_str(), info.bridge_id.as_str()),
        ("pi", "pi-id")
    );
    assert_eq!(info.backend.as_deref(), Some("Simulated"));
    let devices = bridge.devices();
    assert_eq!(devices.len(), 2, "every output of the machine");
    assert_eq!(devices[0].id, "sim-usb-dac");
    assert_eq!(devices[0].max_sample_rate(), 384_000);
    let (offset, uncertainty) = bridge.clock_offset_us().expect("clock estimate");
    assert!(
        offset.abs() < 5_000 && uncertainty < 5_000,
        "{offset} ± {uncertainty}"
    );

    // 192 kHz/24-bit, exclusive: the DAC is opened at exactly that format.
    let format = StreamFormat {
        sample_rate: 192_000,
        sample_format: SampleFormat::S24Packed,
        channels: 2,
    };
    let mut stream = bridge
        .open("sim-usb-dac", format, true)
        .expect("open the DAC");
    let data = noise(192_000 * 2, 3, 5);
    for chunk in data.chunks(4_096 * format.frame_bytes()) {
        stream.write(chunk).expect("write");
    }
    assert!(stream.delay_frames() > 0, "audio is still buffered");
    stream.drain().expect("drain");
    assert_eq!(stream.delay_frames(), 0);
    let capture = backend.capture("sim-usb-dac");
    assert_eq!(capture.opened, vec![(format, true)]);
    assert_eq!(capture.bytes.len(), data.len(), "no padding, no gap");
    assert!(capture.bytes == data, "every sample unchanged");

    // A rate the device does not take is refused, like hw: on ALSA.
    let error = bridge
        .open(
            "sim-hdmi",
            StreamFormat {
                sample_rate: 96_000,
                ..format
            },
            true,
        )
        .err()
        .expect("HDMI output takes 48 kHz only");
    assert!(error.to_string().contains("does not take"), "{error}");

    // Hardware volume and mute reach the device's own mixer.
    bridge
        .set_hardware_volume("sim-usb-dac", -23.5)
        .expect("volume");
    bridge.set_hardware_mute("sim-usb-dac", true).expect("mute");
    let deadline = Instant::now() + Duration::from_secs(2);
    // Volume and mute are separate messages: wait for both to land.
    while Instant::now() < deadline && {
        let capture = backend.capture("sim-usb-dac");
        capture.volume_db != Some(-23.5) || !capture.muted
    } {
        std::thread::sleep(Duration::from_millis(5));
    }
    let capture = backend.capture("sim-usb-dac");
    assert_eq!(capture.volume_db, Some(-23.5));
    assert!(capture.muted);
    drop(stream);

    // Pause holds the position; a flush discards what is buffered.
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    let mut stream = bridge.open("sim-usb-dac", format, true).expect("reopen");
    let second = noise(44_100, 2, 9);
    stream.write(&second).expect("write");
    // Pause returns once the bridge reports its device stopped, so the
    // position it leaves is exact and holds through later reports.
    stream.pause(true).expect("pause in place");
    let held = stream.delay_frames();
    // Several status reports (every 50 ms) arrive while paused.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(stream.delay_frames(), held, "nothing plays while paused");
    stream.pause(false).expect("resume");
    stream.flush().expect("flush");
    assert_eq!(stream.delay_frames(), 0);
    stream
        .write(&second[..4 * 4_410])
        .expect("write after flush");
    stream.drain().expect("drain");

    // Device changes reach the server (a DAC unplugged).
    backend.set_devices(vec![backend.devices()[0].clone()]);
    let deadline = Instant::now() + Duration::from_secs(8);
    while bridge.devices().len() != 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(bridge.devices().len(), 1);
    assert_eq!(endpoint.sessions(), 1);

    // Another Parson server is refused while this one is active.
    let error = RemoteBridge::connect(
        endpoint.address(),
        "server-2",
        "Neighbour",
        Duration::from_secs(5),
    )
    .err()
    .expect("refused");
    assert!(error.contains("in use by another Parson server"), "{error}");
    // The same server reconnecting (after a restart) takes over.
    let again = connect(&endpoint);
    assert_eq!(again.devices().len(), 1);
    let deadline = Instant::now() + Duration::from_secs(3);
    while bridge.connected() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!bridge.connected(), "the old session is closed");
}

#[test]
fn grouped_bridges_play_sample_synced_to_the_clock_master() {
    // The follower's DAC runs 500 ppm fast and its clock reads 3 s ahead.
    let (master_endpoint, master_backend) = endpoint("living-room", 1.0, 0);
    let (follower_endpoint, follower_backend) = endpoint("kitchen", 1.0005, 3_000_000);
    let master = connect(&master_endpoint);
    let follower = connect(&follower_endpoint);
    let (offset, _) = follower.clock_offset_us().expect("offset");
    assert!((offset - 3_000_000).abs() < 5_000, "{offset}");

    let group = SyncGroup::new(
        "group-1",
        "Living Room + Kitchen",
        vec![
            GroupMember {
                bridge: master.clone(),
                device_id: "sim-usb-dac".into(),
            },
            GroupMember {
                bridge: follower.clone(),
                device_id: "sim-usb-dac".into(),
            },
        ],
    );
    let device = group.device().expect("common format");
    assert_eq!(
        device.detail, "living-room + kitchen",
        "machines, master first"
    );
    assert!(device.sample_rates().contains(&44_100));

    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    let (mut stream, probe) = group.open_probe(format);
    let data = noise(44_100 * 4, 2, 21);
    let chunk_bytes = 2_048 * format.frame_bytes();
    let started = Instant::now();
    let mut errors_ms: Vec<f64> = Vec::new();
    for chunk in data.chunks(chunk_bytes) {
        stream.write(chunk).expect("group write");
        // Once the follower has had time to settle, compare positions.
        if started.elapsed() > Duration::from_millis(2_200) {
            let members = probe();
            if members.len() == 2 && members.iter().all(|member| member.progress.started) {
                let difference =
                    members[0].progress.heard as f64 - members[1].progress.heard as f64;
                errors_ms.push(difference.abs() * 1_000.0 / 44_100.0);
            }
        }
    }
    let members = probe();
    assert_eq!(members.len(), 2);
    assert!(members[0].master && !members[1].master);
    assert_eq!(
        members[0].progress.corrections, 0,
        "the master is never touched"
    );
    let corrections = members[1].progress.corrections;
    stream.drain().expect("drain");
    drop(stream);
    assert!(errors_ms.len() > 5, "compared positions while playing");
    errors_ms.sort_by(f64::total_cmp);
    let median = errors_ms[errors_ms.len() / 2];
    // Without correction the fast DAC would be 1.5 ms ahead after 3 s (and
    // 3 s off without the clock offset). Positions come from progress
    // reports, so allow for a late report on a busy test machine.
    assert!(
        median < 0.5,
        "members drifted {median:.3} ms apart: {errors_ms:?}"
    );
    assert!(
        corrections > 15,
        "the fast follower repeated frames to stay on the master: {corrections}"
    );

    // The master played every sample, after the priming silence.
    let capture = master_backend.capture("sim-usb-dac");
    let frame = format.frame_bytes();
    let first_audio = capture
        .bytes
        .chunks(frame)
        .position(|frame| frame.iter().any(|byte| *byte != 0))
        .expect("audio");
    assert!(
        capture.bytes[first_audio * frame..] == data[..],
        "the clock master is bit-perfect"
    );
    // The follower differs only by the single repeated frames.
    let follower_capture = follower_backend.capture("sim-usb-dac");
    let follower_audio = follower_capture.bytes.len() / frame
        - follower_capture
            .bytes
            .chunks(frame)
            .position(|frame| frame.iter().any(|byte| *byte != 0))
            .expect("follower audio");
    // Corrections continue while the last 750 ms play out (about 17 more
    // frames at 500 ppm).
    let inserted = follower_audio as i64 - (data.len() / frame) as i64;
    assert!(
        (corrections..=corrections + 30).contains(&inserted),
        "inserted frames are the only difference: {inserted} vs {corrections}"
    );
}

#[test]
fn groups_take_the_formats_every_member_shares() {
    let backend = SimulatedBackend::with_default_devices(1.0);
    let devices = backend.devices();
    let common = common_device("g", "Group", &devices).expect("shared format");
    assert_eq!(common.sample_rates(), vec![48_000]);
    assert_eq!(common.sample_formats(), vec![SampleFormat::S16]);
    assert!(common.hardware_volume.is_none(), "HDMI has no mixer");
    let mut dac_only = devices[1].clone();
    if let Some(caps) = dac_only.caps.as_mut() {
        caps.sample_rates = vec![44_100];
    }
    assert!(common_device("g", "Group", &[devices[1].clone(), dac_only]).is_none());
}

#[test]
fn a_stream_whose_device_is_taken_over_fails_instead_of_hanging() {
    // Real-time DAC, so the old stream's writes would block on the window.
    let (endpoint, backend) = endpoint("pi", 1.0, 0);
    let bridge = connect(&endpoint);
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    let mut old = bridge.open("sim-usb-dac", format, true).expect("open");
    let second = noise(44_100, 2, 3);
    old.write(&second[..44_100]).expect("first writes play");
    // Something else (a synced group, another zone) opens the same DAC.
    let mut new = bridge
        .open("sim-usb-dac", format, true)
        .expect("the newer stream takes the device");
    let started = Instant::now();
    let mut failed = None;
    for chunk in second.chunks(4_410 * format.frame_bytes()) {
        if let Err(error) = old.write(chunk) {
            failed = Some(error);
            break;
        }
    }
    let error = failed.expect("the replaced stream reports the takeover");
    assert!(
        error.to_string().contains("another stream took over"),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "fails at once: {:?}",
        started.elapsed()
    );
    assert!(
        bridge.recent_errors().is_empty(),
        "a takeover is not a bridge fault: {:?}",
        bridge.recent_errors()
    );
    drop(old);
    // The newer stream plays every sample.
    let data = noise(8_820, 2, 11);
    new.write(&data).expect("the new stream plays");
    new.drain().expect("drain");
    let capture = backend.capture("sim-usb-dac");
    assert!(
        capture.bytes.ends_with(&data),
        "the new stream reached the DAC"
    );
}

#[test]
fn a_late_start_never_costs_the_clock_master_a_sample() {
    // The common start time has already passed when the audio arrives (a
    // slow decoder, a busy machine): the follower skips what it should
    // have played by now, but the master's timeline is the group's, so it
    // starts at once and keeps every frame.
    let (endpoint, backend) = endpoint("living-room", 4.0, 0);
    let bridge = connect(&endpoint);
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    let now = bridge
        .clock_offset_us()
        .map(|(offset, _)| offset)
        .unwrap_or(0)
        + crate::clock::monotonic_us();
    let data = noise(22_050, 2, 5);
    for (device, master) in [("sim-usb-dac", true), ("sim-hdmi", false)] {
        let format = StreamFormat {
            sample_rate: if master { 44_100 } else { 48_000 },
            ..format
        };
        let mut stream = bridge
            .open_stream(
                device,
                format,
                true,
                Some(crate::protocol::SyncStart {
                    // 20 ms ago.
                    start_bridge_us: now - 20_000,
                    master,
                }),
            )
            .expect("open");
        stream.write(&data).expect("write");
        stream.drain().expect("drain");
        let capture = backend.capture(device);
        let frame = format.frame_bytes();
        let first = capture
            .bytes
            .chunks(frame)
            .position(|frame| frame.iter().any(|byte| *byte != 0))
            .expect("audio");
        let played = &capture.bytes[first * frame..];
        if master {
            assert!(played == data.as_slice(), "the master keeps every sample");
        } else {
            let skipped = (data.len() - played.len()) / frame;
            assert!(
                skipped >= 48 * 20 / 2 && data.ends_with(played),
                "a late follower skips ahead: {skipped} frames"
            );
        }
    }
}
