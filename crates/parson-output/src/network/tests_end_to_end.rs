//! End-to-end tests of the network outputs against the protocol fakes:
//! discovery, control, and the samples that arrive, compared exactly.

use std::io::{self, Cursor};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fakes::{
    FakeAirPlayConfig, FakeAirPlayReceiver, FakeRenderer, FakeRendererConfig, sample_icon,
};
use super::raop::RaopBackend;
use super::upnp::UpnpBackend;
use super::{ManualEndpoint, NetworkCapsSource, NetworkProtocol, NetworkRegistry, TrackInfo};
use crate::player::{InputFormat, PcmSource, Player, PlayerState, PlayerStatus};
use crate::{Backend, Driver, SampleFormat, StreamFormat, pack_samples};

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

/// Interleaved left-justified S32 with a recognisable 24-bit ramp.
fn s32_ramp(frames: usize, channels: usize) -> Vec<u8> {
    (0..frames * channels)
        .flat_map(|index| {
            let value = ((index as i32 * 7919) % 8_000_000 - 4_000_000) << 8;
            value.to_le_bytes()
        })
        .collect()
}

fn track() -> TrackInfo {
    TrackInfo {
        title: "Ramp & Roll".into(),
        artist: Some("Parson Bench".into()),
        album: Some("Fixtures".into()),
        duration_ms: Some(1_000),
    }
}

#[test]
fn upnp_renderer_found_by_unicast_search_plays_96_24_bit_exact() {
    let renderer = FakeRenderer::start(FakeRendererConfig {
        speed: 40.0,
        icon: Some(sample_icon()),
        ..FakeRendererConfig::default()
    })
    .unwrap();
    let ssdp = renderer.ssdp_address.unwrap();
    let backend = UpnpBackend::new(registry(NetworkProtocol::OpenHome, ssdp.to_string()), None);
    let devices = backend.devices();
    assert_eq!(devices.len(), 1, "{devices:?}");
    let device = &devices[0];
    assert_eq!(device.driver, Driver::Upnp);
    assert_eq!(device.name, "Parson Test Renderer");
    assert!(device.probe_error.is_none(), "{:?}", device.probe_error);
    let network = device.network.as_ref().unwrap();
    assert_eq!(network.protocol, NetworkProtocol::OpenHome);
    assert_eq!(network.caps_source, NetworkCapsSource::Assumed);
    assert!(network.reports_format);
    assert!(network.manual);
    assert!(network.icon_url.as_deref().unwrap().ends_with("/icon.png"));
    assert!(network.accepts.contains(&"WAV".to_string()));
    assert_eq!(device.caps.as_ref().unwrap().bit_depths(), vec![16, 24]);

    backend.prepare(&device.id, &track());
    let format = StreamFormat {
        sample_rate: 96_000,
        sample_format: SampleFormat::S24Packed,
        channels: 2,
    };
    let pcm = s32_ramp(96_000 / 2, 2);
    let source = pcm.clone();
    let id = device.id.clone();
    let backend = Arc::new(backend);
    let open_backend = Arc::clone(&backend);
    let player = Player::start(
        Box::new(move || open_backend.open(&id, format, true)),
        Box::new(move |_| Ok(Box::new(Cursor::new(source.clone())) as PcmSource)),
        format,
        InputFormat::S32,
        0,
    )
    .unwrap();
    let status = wait(&player, PlayerState::Ended, Duration::from_secs(20));
    assert_eq!(status.state, PlayerState::Ended, "{status:?}");
    let mut expected = Vec::new();
    pack_samples(&pcm, SampleFormat::S24Packed, &mut expected);
    let state = renderer.wait_for(Duration::from_secs(5), |state| state.ended);
    assert_eq!(state.stream_format, Some((96_000, 24, 2)));
    assert_eq!(state.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(state.received.len(), expected.len());
    assert!(
        state.received == expected,
        "the renderer received different samples"
    );
    assert!(
        state
            .actions
            .iter()
            .any(|action| action == "AVTransport#SetAVTransportURI")
    );
    assert!(
        state
            .actions
            .iter()
            .any(|action| action == "AVTransport#Play")
    );
    let metadata = state.metadata.unwrap();
    assert!(
        metadata.contains("<dc:title>Ramp &amp; Roll</dc:title>"),
        "{metadata}"
    );
    assert!(metadata.contains("sampleFrequency=\"96000\""));
}

#[test]
fn openhome_playlist_renderer_reports_what_it_decodes_and_pauses() {
    let renderer = FakeRenderer::start(FakeRendererConfig {
        av_transport: false,
        sink_protocol_info: None,
        ..FakeRendererConfig::default()
    })
    .unwrap();
    let backend = Arc::new(UpnpBackend::new(
        registry(NetworkProtocol::OpenHome, renderer.description_url()),
        None,
    ));
    let device = backend.devices().remove(0);
    assert_eq!(
        device.network.as_ref().unwrap().protocol,
        NetworkProtocol::OpenHome
    );
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
    let id = device.id.clone();
    let open_backend = Arc::clone(&backend);
    let player = Player::start(
        Box::new(move || open_backend.open(&id, format, true)),
        Box::new(|_| Ok(Box::new(io::repeat(0)) as PcmSource)),
        format,
        InputFormat::S16,
        0,
    )
    .unwrap();
    let state = renderer.wait_for(Duration::from_secs(5), |state| !state.received.is_empty());
    assert!(
        state
            .actions
            .iter()
            .any(|action| action == "Playlist#Insert"),
        "{:?}",
        state.actions
    );
    assert!(
        state
            .actions
            .iter()
            .any(|action| action == "Playlist#SeekId")
    );
    let report = backend
        .hardware_status(&device.id, &format)
        .expect("Info.Details report");
    assert!(report.matches_stream, "{report:?}");
    assert_eq!(report.params.rate, 44_100);
    assert_eq!(report.params.format, "WAV 16bit, lossless");
    assert_eq!(report.reporter, crate::procfs::HardwareReporter::Renderer);

    player.pause();
    wait(&player, PlayerState::Paused, Duration::from_secs(5));
    let stopped = renderer.wait_for(Duration::from_secs(5), |state| {
        state.transport_state == "STOPPED"
    });
    assert_eq!(stopped.transport_state, "STOPPED");
    player.resume();
    let resumed = renderer.wait_for(Duration::from_secs(5), |state| {
        state.fetches >= 2 && state.transport_state == "PLAYING" && !state.received.is_empty()
    });
    assert!(resumed.fetches >= 2, "{resumed:?}");
    player.stop();
}

#[test]
fn a_renderer_that_resamples_is_reported_as_converting() {
    let renderer = FakeRenderer::start(FakeRendererConfig {
        reports_rate: Some(48_000),
        ..FakeRendererConfig::default()
    })
    .unwrap();
    let backend = Arc::new(UpnpBackend::new(
        registry(NetworkProtocol::UpnpAv, renderer.description_url()),
        None,
    ));
    let device = backend.devices().remove(0);
    let format = StreamFormat {
        sample_rate: 44_100,
        sample_format: SampleFormat::S24Packed,
        channels: 2,
    };
    let id = device.id.clone();
    let open_backend = Arc::clone(&backend);
    let player = Player::start(
        Box::new(move || open_backend.open(&id, format, true)),
        Box::new(|_| Ok(Box::new(io::repeat(0)) as PcmSource)),
        format,
        InputFormat::S32,
        0,
    )
    .unwrap();
    renderer.wait_for(Duration::from_secs(5), |state| !state.received.is_empty());
    let report = backend.hardware_status(&device.id, &format).unwrap();
    assert!(!report.matches_stream);
    assert_eq!(report.params.rate, 48_000);
    player.stop();
}

#[test]
fn lpcm_only_renderer_gets_big_endian_l16_at_a_declared_rate() {
    let renderer = FakeRenderer::start(FakeRendererConfig {
        openhome: false,
        speed: 40.0,
        sink_protocol_info: Some(
            "http-get:*:audio/L16;rate=44100;channels=2:DLNA.ORG_PN=LPCM,http-get:*:audio/L16;rate=48000;channels=2:*".into(),
        ),
        ..FakeRendererConfig::default()
    })
    .unwrap();
    let backend = Arc::new(UpnpBackend::new(
        registry(NetworkProtocol::UpnpAv, renderer.description_url()),
        None,
    ));
    let device = backend.devices().remove(0);
    let network = device.network.as_ref().unwrap();
    assert_eq!(network.protocol, NetworkProtocol::UpnpAv);
    assert_eq!(network.caps_source, NetworkCapsSource::Declared);
    assert_eq!(
        device.caps.as_ref().unwrap().sample_rates,
        vec![44_100, 48_000]
    );
    let format = StreamFormat {
        sample_rate: 48_000,
        sample_format: SampleFormat::S16,
        channels: 2,
    };
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
            .is_err(),
        "96kHz is not declared"
    );
    let pcm: Vec<u8> = (0..48_000u32 * 2)
        .flat_map(|i| ((i * 31) as i16).to_le_bytes())
        .collect();
    let source = pcm.clone();
    let id = device.id.clone();
    let open_backend = Arc::clone(&backend);
    let player = Player::start(
        Box::new(move || open_backend.open(&id, format, true)),
        Box::new(move |_| Ok(Box::new(Cursor::new(source.clone())) as PcmSource)),
        format,
        InputFormat::S16,
        0,
    )
    .unwrap();
    assert_eq!(
        wait(&player, PlayerState::Ended, Duration::from_secs(20)).state,
        PlayerState::Ended
    );
    let state = renderer.wait_for(Duration::from_secs(5), |state| state.ended);
    assert_eq!(
        state.content_type.as_deref(),
        Some("audio/L16;rate=48000;channels=2")
    );
    let mut expected = Vec::new();
    super::to_big_endian(&pcm, 2, &mut expected);
    assert!(
        state.received == expected,
        "L16 must arrive big endian and exact"
    );
}

#[test]
fn unreachable_manual_renderer_is_listed_with_the_reason() {
    let backend = UpnpBackend::new(
        registry(
            NetworkProtocol::UpnpAv,
            "http://127.0.0.1:9/description.xml".into(),
        ),
        None,
    );
    let devices = backend.devices();
    assert_eq!(devices.len(), 1);
    assert!(devices[0].caps.is_none());
    assert!(
        devices[0]
            .probe_error
            .as_deref()
            .unwrap()
            .contains("could not reach")
    );
}

#[test]
fn airplay_receiver_gets_exact_alac_with_sync_and_timing() {
    let receiver = FakeAirPlayReceiver::start(FakeAirPlayConfig {
        audio_latency: Some(11_025),
        ..FakeAirPlayConfig::default()
    })
    .unwrap();
    let backend = Arc::new(RaopBackend::new(
        registry(NetworkProtocol::AirPlay, receiver.address.to_string()),
        false,
    ));
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
    let state = receiver.wait_for(Duration::from_secs(3), |state| {
        state.torn_down && state.timing_ok
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
    assert!(state.decode_errors.is_empty(), "{:?}", state.decode_errors);
    assert_eq!(state.marker_packets, 1);
    assert_eq!(state.out_of_order, 0);
    assert!(state.first_sync_seen && state.sync_packets >= 1);
    assert!(state.timing_ok, "the sender answers timing requests");
    assert!(state.torn_down);
    assert_eq!(state.packets as usize, frames.div_ceil(352));
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
    let backend = RaopBackend::new(
        registry(NetworkProtocol::AirPlay, receiver.address.to_string()),
        false,
    );
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
