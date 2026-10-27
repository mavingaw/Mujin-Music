//! End-to-end tests of the AirPlay output against the protocol fake:
//! control, timing, sync, resends and the samples that arrive, compared
//! exactly.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fakes::{FakeAirPlayConfig, FakeAirPlayReceiver};
use super::raop::RaopBackend;
use super::{ManualEndpoint, NetworkCapsSource, NetworkProtocol, NetworkRegistry, TrackInfo};
use crate::player::{InputFormat, PcmSource, Player, PlayerState, PlayerStatus};
use crate::{Backend, Driver, SampleFormat, StreamFormat};

fn registry(protocol: NetworkProtocol, address: String) -> Arc<NetworkRegistry> {
    Arc::new(NetworkRegistry::new(vec![ManualEndpoint {
        protocol,
        address,
        name: Some("Bench".into()),
    }]))
}

fn wait(player: &Player, wanted: PlayerState, timeout: Duration) -> PlayerStatus {
    let deadline = Instant::now() + timeout;
    loop {
        let status = player.status();
        if status.state == wanted
            || status.state == PlayerState::Failed
            || Instant::now() > deadline
        {
            return status;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn track() -> TrackInfo {
    TrackInfo {
        title: "Ramp & Roll".into(),
        artist: Some("Mujin Music Bench".into()),
        album: Some("Fixtures".into()),
        duration_ms: Some(1_000),
    }
}

#[test]
fn airplay_receiver_gets_exact_alac_with_sync_and_timing() {
    let receiver = FakeAirPlayReceiver::start(FakeAirPlayConfig {
        audio_latency: Some(11_025),
        drop_packet: Some(5),
        ..FakeAirPlayConfig::default()
    })
    .unwrap();
    let backend = Arc::new(RaopBackend::new(registry(
        NetworkProtocol::AirPlay,
        receiver.address.to_string(),
    )));
    let devices = backend.devices();
    assert_eq!(devices.len(), 1, "{devices:?}");
    let device = devices[0].clone();
    assert_eq!(device.driver, Driver::AirPlay);
    assert!(device.probe_error.is_none(), "{:?}", device.probe_error);
    let caps = device.caps.as_ref().unwrap();
    assert_eq!(caps.sample_rates, vec![44_100]);
    assert_eq!(caps.bit_depths(), vec![16]);
    assert_eq!(
        device.network.as_ref().unwrap().caps_source,
        NetworkCapsSource::Protocol
    );
    backend.prepare(&device.id, &track());
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    // 0.6 s of audio, not a whole number of packets.
    let frames = 26_500usize;
    let samples: Vec<i16> = (0..frames * 2)
        .map(|i| (((i * 37) % 60_000) as i32 - 30_000) as i16)
        .collect();
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let id = device.id.clone();
    let open_backend = Arc::clone(&backend);
    let player = Player::start(
        Box::new(move || open_backend.open(&id, format, true)),
        Box::new(move |_| Ok(Box::new(Cursor::new(pcm.clone())) as PcmSource)),
        format,
        InputFormat::S16,
        0,
    )
    .unwrap();
    let status = wait(&player, PlayerState::Ended, Duration::from_secs(15));
    assert_eq!(status.state, PlayerState::Ended, "{status:?}");
    drop(player);
    // The resent packet travels on its own socket and thread, so on a busy
    // machine it can land after the teardown: wait for it too.
    let state = receiver.wait_for(Duration::from_secs(3), |state| {
        state.torn_down && state.timing_ok && state.resent >= 1
    });
    // The first OPTIONS is the listing's probe of the manual address.
    assert_eq!(
        state.methods[..6],
        [
            "OPTIONS",
            "OPTIONS",
            "ANNOUNCE",
            "SETUP",
            "RECORD",
            "SET_PARAMETER"
        ]
    );
    assert!(state.sdp.unwrap().contains("a=rtpmap:96 AppleLossless"));
    assert!(state.parameters[0].contains("title: Ramp & Roll"));
    // The receiver gets Parson's level as soon as the session starts.
    assert_eq!(state.volumes, vec![-15.0]);
    assert_eq!(device.hardware_volume, Some(super::raop::AIRPLAY_VOLUME));
    assert_eq!(state.sync_latency, Some(11_025));
    assert_eq!(state.resent, 1, "resends the packet the receiver lost");
    assert!(state.decode_errors.is_empty(), "{:?}", state.decode_errors);
    assert_eq!(state.marker_packets, 1);
    assert_eq!(state.out_of_order, 0);
    assert!(state.first_sync_seen && state.sync_packets >= 1);
    assert!(state.timing_ok, "the sender answers timing requests");
    assert!(state.torn_down);
    assert_eq!(state.packets as usize, frames.div_ceil(352));
    assert!(state.dropped_seq.is_some());
    assert_eq!(
        &state.samples[..samples.len()],
        &samples[..],
        "ALAC must carry the samples exactly"
    );
    assert!(
        state.samples[samples.len()..]
            .iter()
            .all(|sample| *sample == 0)
    );
}

#[test]
fn airplay_receiver_that_requires_pairing_fails_with_the_reason() {
    let receiver = FakeAirPlayReceiver::start(FakeAirPlayConfig {
        require_pairing: true,
        ..FakeAirPlayConfig::default()
    })
    .unwrap();
    let backend = RaopBackend::new(registry(
        NetworkProtocol::AirPlay,
        receiver.address.to_string(),
    ));
    let device = backend.devices().remove(0);
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    let error = backend.open(&device.id, format, true).err().unwrap();
    assert!(
        error.to_string().contains("requires AirPlay pairing"),
        "{error}"
    );
    assert!(
        backend
            .open(
                &device.id,
                StreamFormat {
                    sample_rate: 96_000,
                    ..format
                },
                true
            )
            .is_err()
    );
}
