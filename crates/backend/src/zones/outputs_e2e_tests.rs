//! End-to-end tests for the non-Sonos outputs:
//!
//! - a UPnP/DLNA renderer (the fake one answers SSDP and SOAP and fetches
//!   media like hardware): discovery, format limits, transfer, gapless
//!   next-track handoff, hi-res conversion, device-scale volume, queue
//!   edits, the end of the queue, a sleep timer and another app taking over;
//! - local audio-device zones on simulated DACs: bit-perfect exclusive
//!   playback proven sample for sample (gapless across two tracks),
//!   hardware volume in dB, software and fixed volume, an HDMI-style output
//!   that has to convert, the maximum sample rate, DSP, private zones,
//!   transfer both ways and the sleep timer;
//! - the Chromecast zone: limits note, remote commands reaching the cast
//!   session, and the receiver media that is FLAC up to 96 kHz/24-bit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use actix_web::dev::Service as _;
use actix_web::test as actix_test;
use actix_web::{App, HttpMessage, HttpServer, web};

use super::service::{ClientRegistration, Viewer};
use super::*;
use crate::api::auth::Claims;
use crate::domain::{Album, Artist, AudioProperties, Song};
use crate::library::search::SearchIndex;
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::outputs::Outputs;
use crate::outputs::device::DeviceManager;
use crate::outputs::device::simulated::SimulatedBackend;
use crate::outputs::upnp::fake::{FakeRenderer, FakeRendererSpec};
use crate::outputs::upnp::{UpnpConfig, UpnpController};

const USER: i32 = 11;

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run(command: &mut std::process::Command) {
    let status = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed: {command:?}");
}

/// Deterministic 24-bit stereo noise, as little-endian 3-byte samples.
fn raw_s24(frames: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(frames * 6);
    for _ in 0..frames * 2 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // Keep below full scale; use all 24 bits.
        let value = ((state >> 8) as i32 & 0x00ff_ffff) - 0x0080_0000;
        let value = value / 2;
        bytes.extend_from_slice(&value.to_le_bytes()[..3]);
    }
    bytes
}

/// A 24-bit FLAC made from exactly `raw` (s24le stereo at `rate`).
fn flac_from_raw(directory: &Path, name: &str, raw: &[u8], rate: u32) -> PathBuf {
    let raw_path = directory.join(format!("{name}.s24"));
    std::fs::write(&raw_path, raw).expect("raw pcm");
    let path = directory.join(format!("{name}.flac"));
    run(std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "s24le", "-ar"])
        .arg(rate.to_string())
        .args(["-ac", "2", "-i"])
        .arg(&raw_path)
        .args([
            "-c:a",
            "flac",
            "-sample_fmt",
            "s32",
            "-bits_per_raw_sample",
            "24",
        ])
        .arg(&path));
    path
}

/// A sine track in `extension` format.
fn tone(directory: &Path, name: &str, extension: &str, rate: u32, seconds: f32) -> PathBuf {
    let path = directory.join(format!("{name}.{extension}"));
    let mut command = std::process::Command::new("ffmpeg");
    command
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency=440:sample_rate={rate}:duration={seconds}"
        ))
        .args(["-ac", "2"]);
    if extension == "flac" {
        command.args([
            "-c:a",
            "flac",
            "-sample_fmt",
            "s32",
            "-bits_per_raw_sample",
            "24",
        ]);
    } else {
        command.args(["-c:a", "libmp3lame", "-b:a", "192k"]);
    }
    run(command.arg(&path));
    path
}

fn song(id: &str, path: &Path, seconds: f64, codec: &str, rate: u32, bits: Option<u8>) -> Song {
    Song {
        id: id.to_string(),
        name: format!("Track {id}"),
        artist: "Fixture Artist".to_string(),
        path: path.to_string_lossy().to_string(),
        duration: seconds,
        audio: Some(AudioProperties {
            codec: Some(codec.to_string()),
            container: Some(codec.to_string()),
            sample_rate: Some(rate),
            bit_depth: bits,
            channels: Some(2),
            bitrate_kbps: None,
            lossless: Some(codec == "flac"),
        }),
        ..Song::default()
    }
}

fn library(songs: Vec<Song>) -> LibraryCache {
    let album = Album {
        id: "album-1".to_string(),
        name: "Fixture Album".to_string(),
        songs: songs.clone(),
        ..Album::default()
    };
    let artist = Artist {
        id: "artist-1".to_string(),
        name: "Fixture Artist".to_string(),
        albums: vec![album],
        ..Artist::default()
    };
    LibraryCache {
        artists: Arc::new(vec![artist.clone()]),
        search_index: SearchIndex::build(std::slice::from_ref(&artist)).expect("search index"),
        song_map: songs
            .iter()
            .map(|song| {
                (
                    song.id.clone(),
                    ("artist-1".to_string(), "album-1".to_string()),
                )
            })
            .collect(),
        album_genres: HashMap::new(),
        artist_positions: HashMap::from([("artist-1".to_string(), 0)]),
        album_positions: HashMap::from([("album-1".to_string(), (0, 0))]),
        song_positions: songs
            .iter()
            .enumerate()
            .map(|(index, song)| (song.id.clone(), (0, 0, index as u32)))
            .collect(),
        songs_flat: songs.iter().map(|song| song.id.clone()).collect(),
        songs_by_artist: HashMap::from([(
            "artist-1".to_string(),
            (0..songs.len() as u32).collect(),
        )]),
        songs_by_genre: HashMap::new(),
        image_paths: Default::default(),
    }
}

fn web_client(song_ids: Option<Vec<String>>, status: ZoneStatus) -> ClientRegistration {
    ClientRegistration {
        kind: ZoneKind::Web,
        name: "Office browser".to_string(),
        device: "Chrome on Linux".to_string(),
        status,
        song_ids,
        current_position: 0,
        position_ms: 1_500,
        duration_ms: 4_000,
        volume: 70,
        muted: false,
        acknowledged_seq: 0,
        repeat: RepeatMode::Off,
        shuffle: false,
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn ffprobe(path: &Path) -> (u32, String) {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate,codec_name",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .expect("ffprobe");
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_default()
            .to_string()
    };
    (
        field("sample_rate").parse().unwrap_or(0),
        field("codec_name"),
    )
}

/// Drives server-driven zones until `condition` holds (about 10 s max).
async fn drive_until<F: Fn(&ZoneService) -> bool>(service: &ZoneService, condition: F) -> bool {
    for tick in 1..=400 {
        service.drive_hosted(tick).await;
        if condition(service) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

fn zone(service: &ZoneService, cache: &LibraryCache, id: &str) -> Zone {
    service.zone(USER, id, Some(cache)).expect("zone")
}

async fn serve_device_routes(
    service: web::Data<ZoneService>,
    lifecycle: web::Data<LibraryLifecycle>,
) -> (u16, actix_web::dev::ServerHandle) {
    let server = HttpServer::new(move || {
        App::new()
            .app_data(service.clone())
            .app_data(lifecycle.clone())
            .configure(crate::api::zones::configure_device_routes)
    })
    .workers(1)
    .bind(("127.0.0.1", 0))
    .expect("bind test server");
    let port = server.addrs()[0].port();
    let server = server.run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (port, handle)
}

#[actix_web::test]
async fn upnp_renderer_plays_gapless_converts_and_follows_the_queue_end_to_end() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-upnp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let hi_res = tone(&directory, "a", "flac", 96_000, 4.0);
    let ultra = tone(&directory, "b", "flac", 192_000, 4.0);
    let lossy = tone(&directory, "c", "mp3", 44_100, 4.0);
    let extra = tone(&directory, "d", "flac", 96_000, 4.0);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            // Listed as ten minutes so the sleep timer fades mid-track.
            song("song-a", &hi_res, 600.0, "flac", 96_000, Some(24)),
            song("song-b", &ultra, 4.0, "flac", 192_000, Some(24)),
            song("song-c", &lossy, 4.0, "mp3", 44_100, None),
            song("song-d", &extra, 4.0, "flac", 96_000, Some(24)),
        ]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");

    let renderer = FakeRenderer::start(FakeRendererSpec {
        udn: "5f9ec1b3-ed59-1900-4530-00a0deadbeef",
        name: "Living Room Streamer",
        manufacturer: "Acme Audio",
        model: "Stream 9",
        sink: "http-get:*:audio/flac:*,http-get:*:audio/L16;rate=96000;channels=2:*,http-get:*:audio/L24;rate=96000;channels=2:*,http-get:*:audio/mpeg:*",
        volume_max: 30,
        gapless: true,
    })
    .await;
    let controller = Arc::new(
        UpnpController::new(UpnpConfig {
            ssdp_targets: vec![renderer.ssdp_address],
            static_locations: Vec::new(),
            search_timeout: Duration::from_millis(400),
        })
        .expect("controller"),
    );
    let service = web::Data::new(ZoneService::new(None, None));
    service.set_outputs(Arc::new(Outputs {
        upnp: Some(controller.clone()),
        devices: None,
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());
    service.set_sleep_fade(Duration::ZERO);
    let (port, server) =
        serve_device_routes(service.clone(), web::Data::from(lifecycle.clone())).await;
    service.set_port(port);

    // --- SSDP discovery, description, SCPDs and protocol info.
    assert_eq!(controller.discover().await.expect("discovery"), 1);
    let zone_id = "upnp:5f9ec1b3-ed59-1900-4530-00a0deadbeef";
    let listed = zone(&service, &cache, zone_id);
    assert_eq!(listed.kind, ZoneKind::Upnp);
    assert_eq!(listed.name, "Living Room Streamer");
    assert_eq!(listed.device, "Acme Audio Stream 9");
    let output = listed.output.clone().expect("output info");
    assert_eq!((output.max_sample_rate, output.max_bit_depth), (96_000, 24));
    assert_eq!(
        output.note,
        "This renderer plays up to 96 kHz/24-bit. FLAC and MP3 within that reach it untouched. Anything else is sent as FLAC: higher sample rates become the nearest rate it takes in the same family (44.1 or 48 kHz multiples), lossless files keep their own bit depth up to 24-bit, and lossy files (Ogg Vorbis, Opus) are decoded to 24-bit."
    );
    assert!(
        listed
            .notes
            .iter()
            .any(|note| note.contains("-80.0 dB to 0.0 dB"))
    );
    let range = listed
        .volume
        .as_ref()
        .and_then(|volume| volume.device_range.clone())
        .expect("device volume range");
    assert_eq!((range.min, range.max, range.unit), (0.0, 30.0, "steps"));
    assert!(listed.capabilities.transfer_target);
    assert_eq!(
        listed.settings.supports.volume_modes,
        vec![VolumeMode::Hardware, VolumeMode::Fixed]
    );

    // --- Transfer from a web browser: queue and position move over.
    service
        .register_client(
            USER,
            "browser-upnp-1",
            web_client(
                Some(ids(&["song-a", "song-b", "song-c"])),
                ZoneStatus::Playing,
            ),
        )
        .expect("register");
    service
        .transfer(USER, "client:browser-upnp-1", zone_id, Some(&cache))
        .await
        .expect("transfer to renderer");
    let state = renderer.snapshot();
    assert!(state.uri.contains("/api/v1/zones/render/song-a/stream"));
    assert!(state.metadata.contains("<dc:title>Track song-a</dc:title>"));
    assert!(state.metadata.contains("audio/flac"));
    assert!(
        state.next_uri.contains("/zones/render/song-b/"),
        "gapless next is queued"
    );
    assert_eq!(state.calls("Play").len(), 1);
    let seeks = state.calls("Seek");
    assert_eq!(seeks.last().expect("seek")["Target"], "0:00:01");
    let pending = service
        .register_client(
            USER,
            "browser-upnp-1",
            web_client(None, ZoneStatus::Playing),
        )
        .expect("heartbeat");
    assert!(
        pending
            .commands
            .iter()
            .any(|command| command.command == ZoneCommand::Pause),
        "the browser is paused after the transfer"
    );

    // The renderer streams the 96/24 file untouched (bit-perfect).
    assert!(renderer.wait_for(|state| !state.fetches.is_empty()).await);
    let fetch = renderer.snapshot().fetches[0].clone();
    assert_eq!(fetch.status, 200);
    assert_eq!(fetch.content_type.as_deref(), Some("audio/flac"));
    assert_eq!(
        fetch.bytes as u64,
        std::fs::metadata(&hi_res).expect("file").len()
    );
    assert!(
        drive_until(&service, |service| zone(service, &cache, zone_id).status
            == ZoneStatus::Playing)
        .await
    );
    let playing = zone(&service, &cache, zone_id);
    assert_eq!(playing.queue.current_position, 0);
    assert_eq!(playing.source, ZoneSource::Parson);
    let stream = playing
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(stream.bit_perfect);
    assert_eq!(stream.source, "FLAC 96 kHz/24-bit");
    assert_eq!(
        stream.output,
        "Original FLAC file, untouched, to Living Room Streamer"
    );
    assert_eq!(stream.quality, crate::zones::StreamQuality::Lossless);

    // --- Volume on the renderer's own 0..30 scale; mute.
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::SetVolume {
                volume: 50,
                member_id: None,
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("volume");
    assert_eq!(renderer.snapshot().volume, 15);
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::SetMute {
                muted: true,
                member_id: None,
            },
            None,
        )
        .await
        .expect("mute");
    assert!(renderer.snapshot().muted);
    service.drive_hosted(5).await;
    let volume = zone(&service, &cache, zone_id).volume.expect("volume");
    assert_eq!(volume.level, 50);
    assert!(volume.muted);
    assert_eq!(
        volume.device_range.and_then(|range| range.current),
        Some(15.0)
    );

    // --- Gapless: the renderer moves to the queued next track by itself.
    renderer.finish_track();
    assert!(
        drive_until(&service, |service| zone(service, &cache, zone_id)
            .queue
            .current_position
            == 1)
        .await
    );
    assert!(
        renderer
            .snapshot()
            .next_uri
            .contains("/zones/render/song-c/")
    );
    // 192 kHz is above the renderer's limit: served as 96 kHz FLAC.
    assert!(renderer.wait_for(|state| state.fetches.len() >= 2).await);
    let converted = renderer.snapshot().fetches[1].clone();
    assert_eq!(converted.status, 200);
    assert_eq!(converted.content_type.as_deref(), Some("audio/flac"));
    assert_eq!(
        ffprobe(converted.saved.as_deref().expect("saved body")),
        (96_000, "flac".to_string())
    );
    let stream = zone(&service, &cache, zone_id)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(!stream.bit_perfect);
    assert!(stream.output.starts_with("FLAC 96 kHz/24-bit to "));
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.starts_with("Sample rate conversion")
                && step.ends_with("to 96 kHz (linear phase)")),
        "{:?}",
        stream.steps
    );
    assert_eq!(stream.quality, crate::zones::StreamQuality::High);

    // --- Queue edits reach the renderer's next track.
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::Load {
                song_ids: ids(&["song-b", "song-c", "song-d"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    assert!(renderer.snapshot().next_uri.contains("/song-c/"));
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::RemoveQueueItem {
                queue_position: 1,
                song_id: Some("song-c".into()),
            },
            Some(&cache),
        )
        .await
        .expect("remove");
    assert!(renderer.snapshot().next_uri.contains("/song-d/"));
    assert_eq!(
        zone(&service, &cache, zone_id).queue.song_ids,
        ids(&["song-b", "song-d"])
    );
    // Signed URLs only stream songs still in the queue.
    let removed = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
        .get(renderer.snapshot().next_uri.replace("song-d", "song-c"))
        .send()
        .await
        .expect("request");
    assert_eq!(removed.status(), 401);

    // --- The end of the queue: the last track plays out and the zone stops.
    renderer.finish_track();
    assert!(
        drive_until(&service, |service| zone(service, &cache, zone_id)
            .queue
            .current_position
            == 1)
        .await
    );
    assert!(
        renderer.snapshot().next_uri.is_empty(),
        "nothing after the last track"
    );
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::Seek { position_ms: 3_000 },
            None,
        )
        .await
        .expect("seek near the end");
    tokio::time::sleep(Duration::from_millis(300)).await;
    service.drive_hosted(1).await;
    renderer.finish_track();
    assert!(
        drive_until(&service, |service| zone(service, &cache, zone_id).status
            == ZoneStatus::Stopped)
        .await
    );

    // --- Sleep timer: fades, pauses, restores volume. A long track, so the
    // timer does not wait for the track to end.
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::Load {
                song_ids: ids(&["song-a", "song-d"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load again");
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::SetMute {
                muted: false,
                member_id: None,
            },
            None,
        )
        .await
        .expect("unmute");
    assert!(
        drive_until(&service, |service| zone(service, &cache, zone_id).status
            == ZoneStatus::Playing)
        .await
    );
    service
        .set_sleep_timer(USER, zone_id, Some(30))
        .expect("sleep timer");
    service.force_sleep_due(zone_id);
    service.run_sleep_timers().await;
    assert_eq!(renderer.snapshot().transport, "PAUSED_PLAYBACK");
    assert_eq!(
        renderer.snapshot().volume,
        15,
        "volume restored after the fade"
    );
    service.drive_hosted(5).await;
    assert_eq!(zone(&service, &cache, zone_id).status, ZoneStatus::Paused);

    // --- Transfer back to the browser.
    service
        .transfer(USER, zone_id, "client:browser-upnp-1", Some(&cache))
        .await
        .expect("transfer back");
    let pending = service
        .register_client(USER, "browser-upnp-1", web_client(None, ZoneStatus::Paused))
        .expect("heartbeat");
    assert!(pending.commands.iter().any(|command| matches!(
        &command.command,
        ZoneCommand::Load { song_ids, .. } if *song_ids == ids(&["song-a", "song-d"])
    )));
    assert_eq!(renderer.snapshot().transport, "STOPPED");

    // --- Another app takes the renderer over.
    renderer.play_external("http://radio.example/live.mp3");
    service.drive_hosted(5).await;
    let taken = zone(&service, &cache, zone_id);
    assert_eq!(taken.source, ZoneSource::External);

    // --- Fixed volume: the slider goes away and volume commands refuse.
    service
        .update_settings(
            USER,
            zone_id,
            ZoneSettingsPatch {
                volume_mode: Some(VolumeMode::Fixed),
                max_sample_rate: Some(Some(48_000)),
                ..ZoneSettingsPatch::default()
            },
            Viewer::default(),
            Some(&cache),
        )
        .await
        .expect("device setup");
    let fixed = zone(&service, &cache, zone_id);
    assert!(!fixed.capabilities.volume);
    assert_eq!(fixed.output.expect("output").max_sample_rate, 48_000);
    assert_eq!(
        service
            .command(
                USER,
                zone_id,
                ZoneCommand::AdjustVolume {
                    delta: 2,
                    member_id: None
                },
                None
            )
            .await,
        Err(ZoneError::Unsupported("fixed_volume"))
    );
    assert_eq!(
        service
            .update_settings(
                USER,
                zone_id,
                ZoneSettingsPatch {
                    exclusive: Some(false),
                    ..ZoneSettingsPatch::default()
                },
                Viewer::default(),
                None,
            )
            .await,
        Err(ZoneError::Unsupported(
            "exclusive_mode_is_for_audio_devices"
        ))
    );

    server.stop(true).await;
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn audio_device_zones_play_bit_perfect_and_follow_device_setup_end_to_end() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-devices-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    // Two 96 kHz/24-bit tracks made from known samples.
    let raw_a = raw_s24(96_000 * 3 / 2, 7);
    let raw_b = raw_s24(96_000, 99);
    let a = flac_from_raw(&directory, "a", &raw_a, 96_000);
    let b = flac_from_raw(&directory, "b", &raw_b, 96_000);
    let cd = tone(&directory, "c", "flac", 44_100, 2.0);
    let long = tone(&directory, "d", "flac", 96_000, 30.0);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            song("song-a", &a, 1.5, "flac", 96_000, Some(24)),
            song("song-b", &b, 1.0, "flac", 96_000, Some(24)),
            song("song-c", &cd, 2.0, "flac", 44_100, Some(24)),
            // Listed as ten minutes so the sleep timer fades mid-track.
            song("song-d", &long, 600.0, "flac", 96_000, Some(24)),
        ]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");

    // Simulated DACs consume audio at twice real time.
    let backend = Arc::new(SimulatedBackend::with_default_devices(2.0));
    let devices = Arc::new(DeviceManager::new(backend.clone(), None));
    let service = ZoneService::new(None, None);
    service.set_outputs(Arc::new(Outputs {
        upnp: None,
        devices: Some(devices.clone()),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());
    service.set_sleep_fade(Duration::ZERO);

    let dac = "device:sim-usb-dac";
    let hdmi = "device:sim-hdmi";
    let zones = service.list(USER, Some(&cache));
    let dac_zone = zones.iter().find(|zone| zone.id == dac).expect("DAC zone");
    assert!(
        zones.iter().any(|zone| zone.id == hdmi),
        "each output is its own zone"
    );
    assert_eq!(dac_zone.kind, ZoneKind::Device);
    assert_eq!(dac_zone.name, "USB DAC (simulated)");
    let range = dac_zone
        .volume
        .as_ref()
        .and_then(|volume| volume.device_range.clone())
        .expect("hardware volume range");
    assert_eq!(
        (range.min, range.max, range.step, range.unit),
        (-127.5, 0.0, 0.5, "dB")
    );
    assert!(range.hardware);
    let output = dac_zone.output.clone().expect("output");
    assert_eq!(
        (output.max_sample_rate, output.max_bit_depth),
        (384_000, 32)
    );
    assert_eq!(output.exclusive, Some(true));
    assert!(dac_zone.settings.supports.exclusive && dac_zone.settings.supports.private);
    assert_eq!(
        dac_zone.settings.supports.volume_modes,
        vec![
            VolumeMode::Hardware,
            VolumeMode::Software,
            VolumeMode::Fixed
        ]
    );
    let hdmi_zone = zones
        .iter()
        .find(|zone| zone.id == hdmi)
        .expect("HDMI zone");
    assert_eq!(hdmi_zone.settings.volume_mode, VolumeMode::Software);
    assert!(
        hdmi_zone
            .output
            .as_ref()
            .expect("output")
            .note
            .contains("48 kHz/16-bit")
    );

    // --- Bit-perfect, gapless: two tracks, one exclusive stream, every sample.
    service
        .command(
            USER,
            dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-a", "song-b"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load on the DAC");
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Playing)
        .await
    );
    let stream = zone(&service, &cache, dac)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(stream.bit_perfect, "{stream:?}");
    assert_eq!(stream.source, "FLAC 96 kHz/24-bit");
    assert_eq!(
        stream.output,
        "96 kHz/24-bit (S24_3LE) to USB DAC (simulated), exclusive"
    );
    assert!(
        drive_until(&service, |service| {
            let zone = zone(service, &cache, dac);
            zone.queue.current_position == 1 && zone.status == ZoneStatus::Stopped
        })
        .await,
        "plays both tracks, then stops at the end of the queue"
    );
    let capture = backend.capture("sim-usb-dac");
    assert_eq!(
        capture.opened.len(),
        1,
        "one stream for both tracks: gapless"
    );
    let (format, exclusive) = capture.opened[0];
    assert!(exclusive);
    assert_eq!(format.sample_rate, 96_000);
    assert_eq!(
        format.format,
        crate::outputs::device::SampleFormat::S24Packed
    );
    let mut expected = raw_a.clone();
    expected.extend_from_slice(&raw_b);
    assert_eq!(capture.bytes.len(), expected.len(), "no padding, no gap");
    assert!(
        capture.bytes == expected,
        "every sample reaches the DAC unchanged"
    );

    // --- Hardware volume in dB on the device's mixer.
    service
        .command(
            USER,
            dac,
            ZoneCommand::SetVolume {
                volume: 80,
                member_id: None,
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("volume");
    assert_eq!(backend.capture("sim-usb-dac").volume_db, Some(-16.0));
    let volume = zone(&service, &cache, dac).volume.expect("volume");
    assert_eq!(volume.level, 80);
    assert_eq!(
        volume.device_range.and_then(|range| range.current),
        Some(-16.0)
    );
    service
        .command(
            USER,
            dac,
            ZoneCommand::SetMute {
                muted: true,
                member_id: None,
            },
            None,
        )
        .await
        .expect("mute");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(backend.capture("sim-usb-dac").muted);
    service
        .command(
            USER,
            dac,
            ZoneCommand::SetMute {
                muted: false,
                member_id: None,
            },
            None,
        )
        .await
        .expect("unmute");

    // --- The HDMI output cannot take 44.1 kHz: Parson converts and says so.
    service
        .command(
            USER,
            hdmi,
            ZoneCommand::Load {
                song_ids: ids(&["song-c"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load on HDMI");
    assert!(
        drive_until(&service, |service| zone(service, &cache, hdmi)
            .output
            .and_then(|output| output.stream)
            .is_some())
        .await
    );
    let stream = zone(&service, &cache, hdmi)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(!stream.bit_perfect);
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step == "Resampled 44.1 kHz to 48 kHz (the device does not take 44.1 kHz)")
    );
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.starts_with("Reduced to 16-bit"))
    );
    assert!(
        drive_until(&service, |service| zone(service, &cache, hdmi).status
            == ZoneStatus::Stopped)
        .await
    );
    let hdmi_capture = backend.capture("sim-hdmi");
    assert_eq!(hdmi_capture.opened[0].0.sample_rate, 48_000);

    // --- Device setup: a 48 kHz maximum, shared mode and DSP on the DAC.
    service
        .update_settings(
            USER,
            dac,
            ZoneSettingsPatch {
                max_sample_rate: Some(Some(48_000)),
                exclusive: Some(false),
                dsp: Some(ZoneDsp {
                    enabled: true,
                    bass_db: 3,
                    crossfeed: true,
                    ..ZoneDsp::default()
                }),
                ..ZoneSettingsPatch::default()
            },
            Viewer::default(),
            Some(&cache),
        )
        .await
        .expect("device setup");
    service
        .command(
            USER,
            dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-b"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac)
            .output
            .and_then(|output| output.stream)
            .is_some_and(|stream| stream.output.contains("48 kHz")))
        .await
    );
    let stream = zone(&service, &cache, dac)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(!stream.bit_perfect);
    assert!(stream.output.ends_with("shared"));
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step == "DSP: Preamp \u{2212}3 dB, Bass +3 dB, Crossfeed")
    );
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Stopped)
        .await
    );
    let opened = backend.capture("sim-usb-dac").opened;
    assert_eq!(
        opened
            .last()
            .map(|(format, exclusive)| (format.sample_rate, *exclusive)),
        Some((48_000, false))
    );

    // Software volume is labelled; fixed volume refuses volume commands.
    service
        .update_settings(
            USER,
            dac,
            ZoneSettingsPatch {
                volume_mode: Some(VolumeMode::Fixed),
                max_sample_rate: Some(None),
                exclusive: Some(true),
                dsp: Some(ZoneDsp::default()),
                ..ZoneSettingsPatch::default()
            },
            Viewer::default(),
            None,
        )
        .await
        .expect("fixed volume");
    assert_eq!(
        service
            .command(
                USER,
                dac,
                ZoneCommand::SetVolume {
                    volume: 10,
                    member_id: None,
                    above_comfort: false,
                },
                None
            )
            .await,
        Err(ZoneError::Unsupported("fixed_volume"))
    );
    assert!(!zone(&service, &cache, dac).capabilities.volume);

    // --- Private: only this computer sees the DAC zone.
    let remote = Viewer::default();
    let local = Viewer {
        device: None,
        local: true,
    };
    assert_eq!(
        service
            .update_settings(
                USER,
                dac,
                ZoneSettingsPatch {
                    private: Some(true),
                    ..ZoneSettingsPatch::default()
                },
                remote,
                None,
            )
            .await,
        Err(ZoneError::Unsupported("set_private_on_the_device_itself"))
    );
    service
        .update_settings(
            USER,
            dac,
            ZoneSettingsPatch {
                private: Some(true),
                ..ZoneSettingsPatch::default()
            },
            local,
            None,
        )
        .await
        .expect("private from this computer");
    assert!(
        service
            .list_for(USER, remote, None)
            .iter()
            .all(|zone| zone.id != dac)
    );
    assert!(
        service
            .list_for(USER, local, None)
            .iter()
            .any(|zone| zone.id == dac)
    );
    assert_eq!(
        service
            .group(USER, &[dac.to_string(), hdmi.to_string()])
            .await,
        Err(ZoneError::Unsupported("private_zones_cannot_be_grouped"))
    );
    service
        .update_settings(
            USER,
            dac,
            ZoneSettingsPatch {
                private: Some(false),
                volume_mode: Some(VolumeMode::Hardware),
                ..ZoneSettingsPatch::default()
            },
            local,
            None,
        )
        .await
        .expect("public again");

    // --- Transfer from a browser to the DAC and back, with position.
    service
        .register_client(
            USER,
            "browser-device-1",
            ClientRegistration {
                position_ms: 4_000,
                duration_ms: 30_000,
                ..web_client(Some(ids(&["song-d", "song-a"])), ZoneStatus::Playing)
            },
        )
        .expect("register");
    service
        .transfer(USER, "client:browser-device-1", dac, Some(&cache))
        .await
        .expect("transfer to the DAC");
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Playing)
        .await
    );
    let moved = zone(&service, &cache, dac);
    assert_eq!(moved.queue.song_ids, ids(&["song-d", "song-a"]));
    assert!(moved.position_ms >= 4_000, "starts where the browser was");

    // --- Linked group across ecosystems is labelled as not sample-synced.
    let linked = service
        .group(
            USER,
            &[dac.to_string(), "client:browser-device-1".to_string()],
        )
        .await
        .expect("link");
    let group = zone(&service, &cache, &linked).group.expect("group");
    assert!(!group.synced);
    assert_eq!(group.sync_note, LINKED_SYNC_NOTE);
    service.ungroup(USER, &linked, None).await.expect("ungroup");

    // --- Sleep timer pauses the DAC zone (a long track: no waiting for its end).
    service
        .set_sleep_timer(USER, dac, Some(15))
        .expect("sleep timer");
    service.force_sleep_due(dac);
    service.run_sleep_timers().await;
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Paused)
        .await
    );
    assert_eq!(
        backend.capture("sim-usb-dac").volume_db,
        Some(-16.0),
        "volume restored"
    );

    // --- Pause all and transfer back.
    service
        .command(USER, dac, ZoneCommand::Play, Some(&cache))
        .await
        .expect("resume");
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Playing)
        .await
    );
    assert_eq!(
        service.pause_all(USER, local, Some(&cache)).await,
        2,
        "the DAC and the browser"
    );
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Paused)
        .await
    );
    service
        .transfer(USER, dac, "client:browser-device-1", Some(&cache))
        .await
        .expect("transfer back");
    let pending = service
        .register_client(
            USER,
            "browser-device-1",
            web_client(None, ZoneStatus::Paused),
        )
        .expect("heartbeat");
    assert!(pending.commands.iter().any(|command| matches!(
        &command.command,
        ZoneCommand::Load { song_ids, position_ms, .. }
            if *song_ids == ids(&["song-d", "song-a"]) && *position_ms >= 4_000
    )));
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Stopped)
        .await
    );

    // --- Unplugging the DAC removes its zone.
    backend.set_devices(Vec::new());
    devices.refresh();
    assert!(service.zone(USER, dac, None).is_none());
    let _ = std::fs::remove_dir_all(&directory);
}

fn claims() -> Claims {
    Claims {
        sub: USER.to_string(),
        exp: usize::MAX,
        username: "listener".to_string(),
        bitrate: 0,
        token_type: "access".to_string(),
        role: "user".to_string(),
        token_version: 0,
        session_id: None,
    }
}

/// The Chromecast zone: a simulated sender (what the mobile app does)
/// opens a cast session, the zone API lists it with its limits and sends
/// it commands, and the simulated receiver fetches FLAC up to 96/24.
#[actix_web::test]
async fn chromecast_zone_reports_limits_takes_commands_and_serves_receiver_flac() {
    use diesel_migrations::MigrationHarness;
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-cast-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let hi_res = tone(&directory, "a", "flac", 96_000, 2.0);
    let ultra = tone(&directory, "b", "flac", 192_000, 2.0);
    let lifecycle = web::Data::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            song("song-a", &hi_res, 2.0, "flac", 96_000, Some(24)),
            song("song-b", &ultra, 2.0, "flac", 192_000, Some(24)),
        ]))
        .await;
    use diesel::r2d2::{ConnectionManager, Pool};
    let pool: crate::persistence::connection::DbPool = Arc::new(
        Pool::builder()
            .max_size(1)
            .build(ConnectionManager::<diesel::sqlite::SqliteConnection>::new(
                ":memory:",
            ))
            .expect("pool"),
    );
    pool.get()
        .expect("connection")
        .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
        .expect("migrations");
    diesel::RunQueryDsl::execute(
        diesel::sql_query(
            "INSERT INTO user(id, username, password) VALUES (11, 'listener', 'hash')",
        ),
        &mut pool.get().expect("connection"),
    )
    .expect("user");
    let service = web::Data::new(ZoneService::new(None, None));
    let app = actix_test::init_service(
        App::new()
            .app_data(service.clone())
            .app_data(lifecycle.clone())
            .app_data(web::Data::new(pool.clone()))
            .wrap_fn(|request, next| {
                request.extensions_mut().insert(claims());
                next.call(request)
            })
            .configure(crate::api::cast::configure)
            .configure(crate::api::zones::configure)
            .route(
                "/api/v1/cast/media/{song}/stream",
                web::get().to(crate::api::cast::cast_media),
            ),
    )
    .await;

    // The phone (sender) starts a cast session.
    let created = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri("/cast/sessions")
            .set_json(serde_json::json!({
                "receiver_id": "kitchen-speaker",
                "receiver_name": "Kitchen display",
                "song_ids": ["song-a", "song-b"],
                "current_position": 0
            }))
            .to_request(),
    )
    .await;
    let status = created.status();
    let body = actix_test::read_body(created).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let session: serde_json::Value = serde_json::from_slice(&body).expect("session json");
    let session_id = session["id"].as_str().expect("session id").to_string();
    assert_eq!(session["items"][0]["content_type"], "audio/flac");
    assert_eq!(session["items"][1]["content_type"], "audio/flac");

    // The zone list shows it with the receiver's format note.
    let listed: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get().uri("/zones").to_request(),
    )
    .await;
    let cast = listed["zones"]
        .as_array()
        .expect("zones")
        .iter()
        .find(|zone| zone["kind"] == "chromecast")
        .expect("cast zone")
        .clone();
    assert_eq!(cast["name"], "Kitchen display");
    assert_eq!(cast["output"]["max_sample_rate"], 96_000);
    assert_eq!(
        cast["output"]["note"],
        "Chromecast plays up to 96 kHz/24-bit. FLAC, MP3 and AAC within that reach it untouched. Anything else is sent as FLAC: higher sample rates become the nearest rate it takes in the same family (44.1 or 48 kHz multiples), lossless files keep their own bit depth up to 24-bit, and lossy files (Ogg Vorbis, Opus) are decoded to 24-bit."
    );

    // Remote control from the zone API reaches the session the sender polls.
    let accepted = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/cast:{session_id}/commands"))
            .set_json(serde_json::json!({"command": "set_volume", "volume": 42}))
            .to_request(),
    )
    .await;
    assert_eq!(accepted.status(), 202);
    let polled: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri(&format!("/cast/sessions/{session_id}"))
            .to_request(),
    )
    .await;
    assert_eq!(polled["command"], "set_volume");
    assert!((polled["command_volume"].as_f64().expect("volume") - 0.42).abs() < 1e-9);

    // The receiver fetches its media: 96/24 FLAC untouched, 192 kHz as 96 kHz.
    let fetch = |index: usize| {
        let url = polled["items"][index]["media_url"]
            .as_str()
            .expect("media url")
            .to_string();
        let path = url
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once('/'))
            .map(|(_, path)| format!("/{path}"))
            .expect("path");
        actix_test::TestRequest::get().uri(&path).to_request()
    };
    let original = actix_test::call_service(&app, fetch(0)).await;
    assert_eq!(original.status(), 200);
    assert_eq!(
        original.headers().get("content-type").expect("type"),
        "audio/flac"
    );
    let body = actix_test::read_body(original).await;
    assert_eq!(
        body.len() as u64,
        std::fs::metadata(&hi_res).expect("file").len()
    );
    let converted = actix_test::call_service(&app, fetch(1)).await;
    assert_eq!(converted.status(), 200);
    assert_eq!(
        converted.headers().get("content-type").expect("type"),
        "audio/flac"
    );
    let body = actix_test::read_body(converted).await;
    let saved = directory.join("received.flac");
    std::fs::write(&saved, &body).expect("save");
    assert_eq!(ffprobe(&saved), (96_000, "flac".to_string()));

    // Pause all reaches the cast session too.
    let paused: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri("/zones/pause-all")
            .to_request(),
    )
    .await;
    assert!(paused["paused"].is_number());
    let _ = std::fs::remove_dir_all(&directory);
}

/// Manual helper: runs a fake UPnP renderer until killed and prints its
/// description URL for `PARSON_UPNP_RENDERERS`.
#[actix_web::test]
#[ignore = "manual: serves a fake UPnP renderer until interrupted"]
async fn serve_fake_upnp_renderer() {
    let renderer = FakeRenderer::start(FakeRendererSpec {
        udn: "5f9ec1b3-ed59-1900-4530-00a0deadbeef",
        name: "Living Room Streamer",
        manufacturer: "Acme Audio",
        model: "Stream 9",
        sink: "http-get:*:audio/flac:*,http-get:*:audio/L16;rate=96000;channels=2:*,http-get:*:audio/L24;rate=96000;channels=2:*,http-get:*:audio/mpeg:*,http-get:*:audio/mp4:*",
        volume_max: 30,
        gapless: true,
    })
    .await;
    println!("PARSON_UPNP_RENDERERS={}", renderer.location);
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Like hardware: a track that has played its length ends.
        let ended = {
            let state = renderer.snapshot();
            let duration = roxmltree::Document::parse(&state.metadata)
                .ok()
                .and_then(|document| {
                    document
                        .descendants()
                        .find(|node| node.tag_name().name() == "res")
                        .and_then(|node| node.attribute("duration"))
                        .and_then(crate::sonos::xml::parse_duration)
                })
                .unwrap_or(0);
            state.transport == "PLAYING" && duration > 0 && state.live_position_ms() >= duration
        };
        if ended {
            renderer.finish_track();
        }
    }
}

/// A Parson Bridge on another machine (here: an endpoint on a loopback
/// socket with simulated DACs).
fn bridge_endpoint(name: &str) -> (parson_bridge::endpoint::Endpoint, Arc<SimulatedBackend>) {
    let backend = Arc::new(SimulatedBackend::with_default_devices(1.0));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
    let endpoint = parson_bridge::endpoint::Endpoint::start(
        listener,
        Some(backend.clone() as Arc<dyn crate::outputs::device::DeviceBackend>),
        parson_bridge::endpoint::EndpointConfig {
            bridge_id: format!("{name}-id"),
            name: name.to_string(),
            host: format!("{name}.local"),
            clock_offset_us: if name == "kitchen" { 2_500_000 } else { 0 },
            note: None,
        },
    )
    .expect("bridge endpoint");
    (endpoint, backend)
}

/// Strips the silence a synced stream starts with.
fn after_priming(bytes: &[u8], frame: usize) -> &[u8] {
    let first = bytes
        .chunks(frame)
        .position(|frame| frame.iter().any(|byte| *byte != 0))
        .unwrap_or(0);
    &bytes[first * frame..]
}

/// Where two captures part ways, in frames.
fn describe_difference(actual: &[u8], expected: &[u8], frame: usize) -> String {
    let first = actual
        .chunks(frame)
        .zip(expected.chunks(frame))
        .position(|(a, b)| a != b);
    let shifted = first.and_then(|index| {
        (1..=4_410).find_map(|shift: usize| {
            let a = actual.get(index * frame..)?;
            let b = expected.get((index + shift) * frame..)?;
            let n = a.len().min(b.len()).min(64 * frame);
            (n > 0 && a[..n] == b[..n]).then(|| format!("{shift} frames dropped"))
        })
    });
    format!(
        "{} frames played, {} expected, first difference at frame {:?} ({})",
        actual.len() / frame,
        expected.len() / frame,
        first,
        shifted.unwrap_or_else(|| "not a simple drop".into())
    )
}

#[actix_web::test]
async fn bridge_zones_play_remote_dacs_bit_perfect_and_group_sample_synced() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-bridges-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let raw_a = raw_s24(96_000 * 3 / 2, 17);
    let raw_c = raw_s24(44_100 * 2, 23);
    let a = flac_from_raw(&directory, "a", &raw_a, 96_000);
    let c = flac_from_raw(&directory, "c", &raw_c, 44_100);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            song("song-a", &a, 1.5, "flac", 96_000, Some(24)),
            song("song-c", &c, 2.0, "flac", 44_100, Some(24)),
        ]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");

    let (kitchen, kitchen_dacs) = bridge_endpoint("kitchen");
    let (living, living_dacs) = bridge_endpoint("living-room");
    let hub = crate::outputs::bridge::BridgeHub::new("server-id".into(), "Test".into(), None);
    let added = hub
        .add(&kitchen.address().to_string())
        .expect("add the kitchen bridge");
    assert!(added.connected, "{added:?}");
    assert_eq!(added.name.as_deref(), Some("kitchen"));
    assert_eq!(added.devices, 2);
    let offset = added.clock_offset_us.expect("clock offset");
    assert!((offset - 2_500_000).abs() < 10_000, "{offset}");
    hub.add(&living.address().to_string())
        .expect("add the living room bridge");
    let service = ZoneService::new(None, None);
    service.set_outputs(Arc::new(Outputs {
        bridges: Some(hub.clone()),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());

    let kitchen_dac = "bridge:kitchen-id:sim-usb-dac";
    let living_dac = "bridge:living-room-id:sim-usb-dac";
    let zones = service.list(USER, Some(&cache));
    assert_eq!(
        zones
            .iter()
            .filter(|zone| zone.kind == ZoneKind::Bridge)
            .count(),
        4,
        "every output of both machines is a zone"
    );
    let dac_zone = zones
        .iter()
        .find(|zone| zone.id == kitchen_dac)
        .expect("kitchen DAC zone");
    assert_eq!(dac_zone.ecosystem, "bridge");
    assert_eq!(
        dac_zone.device,
        "kitchen · Parson Bridge · Simulated USB Audio 2.0 DAC"
    );
    assert_eq!(
        dac_zone
            .output
            .as_ref()
            .map(|output| output.max_sample_rate),
        Some(384_000)
    );
    assert_eq!(dac_zone.settings.volume_mode, VolumeMode::Hardware);
    assert!(dac_zone.settings.supports.exclusive);
    assert!(
        dac_zone
            .notes
            .iter()
            .any(|note| note.contains("not yet on real hardware"))
    );

    // --- A remote DAC: exclusive, at the file's own 96 kHz/24-bit.
    service
        .command(
            USER,
            kitchen_dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-a"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load on the kitchen DAC");
    assert!(
        drive_until(&service, |service| zone(service, &cache, kitchen_dac)
            .status
            == ZoneStatus::Playing)
        .await
    );
    let stream = zone(&service, &cache, kitchen_dac)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(stream.bit_perfect, "{stream:?}");
    assert_eq!(
        stream.output,
        "96 kHz/24-bit (S24_3LE) to USB DAC (simulated) on kitchen via Parson Bridge, exclusive"
    );
    assert!(
        drive_until(&service, |service| zone(service, &cache, kitchen_dac)
            .status
            == ZoneStatus::Stopped)
        .await,
        "plays to the end"
    );
    let capture = kitchen_dacs.capture("sim-usb-dac");
    assert_eq!(capture.opened.len(), 1);
    assert_eq!(capture.opened[0].0.sample_rate, 96_000);
    assert!(capture.opened[0].1, "opened exclusively");
    assert!(
        capture.bytes == raw_a,
        "every sample reaches the remote DAC unchanged"
    );

    // --- Hardware volume on the remote DAC's own mixer, in dB.
    service
        .command(
            USER,
            kitchen_dac,
            ZoneCommand::SetVolume {
                volume: 80,
                member_id: None,
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("volume");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(kitchen_dacs.capture("sim-usb-dac").volume_db, Some(-16.0));

    // --- Group the two machines: one sample-synced zone, master first.
    service
        .command(
            USER,
            living_dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-c"]),
                current_position: 0,
                position_ms: 0,
                play: false,
            },
            Some(&cache),
        )
        .await
        .expect("queue on the living room");
    let group_id = service
        .group(USER, &ids(&[living_dac, kitchen_dac]))
        .await
        .expect("group bridge zones");
    assert!(group_id.starts_with("bridge-group:"), "{group_id}");
    let zones = service.list(USER, Some(&cache));
    assert!(
        !zones
            .iter()
            .any(|zone| zone.id == living_dac || zone.id == kitchen_dac),
        "members fold into the group"
    );
    let group = zone(&service, &cache, &group_id);
    assert_eq!(group.name, "living-room + kitchen", "named by machine");
    let info = group.group.clone().expect("group info");
    assert!(info.synced);
    assert!(info.sync_note.contains("sample-synced"));
    assert_eq!(
        info.members
            .iter()
            .map(|member| member.id.as_str())
            .collect::<Vec<_>>(),
        vec![living_dac, kitchen_dac]
    );
    assert_eq!(
        group.queue.song_ids,
        ids(&["song-c"]),
        "the first zone's queue carries on"
    );
    service
        .command(USER, &group_id, ZoneCommand::Play, Some(&cache))
        .await
        .expect("play the group");
    assert!(
        drive_until(&service, |service| zone(service, &cache, &group_id).status
            == ZoneStatus::Playing)
        .await
    );
    let stream = zone(&service, &cache, &group_id)
        .output
        .and_then(|output| output.stream)
        .expect("group signal path");
    assert!(
        !stream.bit_perfect,
        "followers are corrected, so not every zone is bit-perfect"
    );
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.contains("clock master")),
        "{stream:?}"
    );
    // Per-zone volume inside the group.
    service
        .command(
            USER,
            &group_id,
            ZoneCommand::SetVolume {
                volume: 50,
                member_id: Some(kitchen_dac.to_string()),
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("member volume");
    assert!(
        drive_until(&service, |service| zone(service, &cache, &group_id).status
            == ZoneStatus::Stopped)
        .await,
        "the group plays to the end"
    );
    assert_eq!(kitchen_dacs.capture("sim-usb-dac").volume_db, Some(-40.0));
    // The clock master got every sample, after the start-up silence.
    let living_capture = living_dacs.capture("sim-usb-dac");
    let frame = 6;
    let master_audio = after_priming(&living_capture.bytes, frame);
    assert!(
        master_audio == raw_c.as_slice(),
        "the clock master is bit-perfect: {}",
        describe_difference(master_audio, &raw_c, frame)
    );
    let kitchen_capture = kitchen_dacs.capture("sim-usb-dac");
    let kitchen_group = after_priming(&kitchen_capture.bytes[raw_a.len()..], frame);
    let difference = kitchen_group.len() as i64 / frame as i64 - raw_c.len() as i64 / frame as i64;
    assert!(
        difference.abs() <= 44,
        "the follower played the same audio within a millisecond: {difference} frames"
    );

    // --- Ungroup: the queue carries on in the first zone.
    service
        .ungroup(USER, &group_id, None)
        .await
        .expect("ungroup");
    assert!(service.zone(USER, &group_id, None).is_none());
    let living = zone(&service, &cache, living_dac);
    assert_eq!(living.queue.song_ids, ids(&["song-c"]));
    assert!(service.zone(USER, kitchen_dac, None).is_some());
    let status = hub.status();
    assert_eq!(status.len(), 2);
    assert!(status.iter().all(|bridge| bridge.connected));
    assert!(hub.remove(&kitchen.address().to_string()));
    assert!(
        service.zone(USER, kitchen_dac, None).is_none(),
        "a removed bridge's zones go"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A bridge on another machine, with any audio backend.
fn bridge_with(
    name: &str,
    backend: Arc<dyn crate::outputs::device::DeviceBackend>,
) -> parson_bridge::endpoint::Endpoint {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
    parson_bridge::endpoint::Endpoint::start(
        listener,
        Some(backend),
        parson_bridge::endpoint::EndpointConfig {
            bridge_id: format!("{name}-id"),
            name: name.to_string(),
            host: format!("{name}.local"),
            clock_offset_us: 0,
            note: None,
        },
    )
    .expect("bridge endpoint")
}

#[actix_web::test]
async fn dacs_on_windows_and_mac_bridges_are_bit_perfect_zones() {
    use parson_bridge::audio::coreaudio::fake::{FakeHal, FakeHalDevice};
    use parson_bridge::audio::coreaudio::{CoreAudioBackend, HalApi};
    use parson_bridge::audio::wasapi::WasapiBackend;
    use parson_bridge::audio::wasapi::fake::{FakeEndpoint, FakeWasapi};

    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory =
        std::env::temp_dir().join(format!("parson-os-bridges-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let raw = raw_s24(96_000, 31);
    let file = flac_from_raw(&directory, "hires", &raw, 96_000);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![song(
            "song-hires",
            &file,
            1.0,
            "flac",
            96_000,
            Some(24),
        )]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");

    // A Windows PC with a USB DAC (WASAPI) and a Mac with one (CoreAudio).
    let guid = "0d0d0d0d-1111-4222-8333-444455556666";
    let wasapi = Arc::new(WasapiBackend::new(FakeWasapi::new(
        vec![FakeEndpoint::usb_dac(guid, "Topping E30 II")],
        1.0,
    )));
    let windows = bridge_with("study-pc", wasapi.clone());
    let hal = Arc::new(CoreAudioBackend::new(FakeHal::new(
        vec![FakeHalDevice::usb_dac(
            90,
            "AppleUSBAudioEngine:Chord:Qutest:1",
            "Qutest",
        )],
        1.0,
    )));
    let mac = bridge_with("studio-mac", hal.clone());
    let hub = crate::outputs::bridge::BridgeHub::new("server-id".into(), "Test".into(), None);
    for endpoint in [&windows, &mac] {
        let added = hub
            .add(&endpoint.address().to_string())
            .expect("add bridge");
        assert!(added.connected, "{added:?}");
        assert_eq!(added.devices, 1);
    }
    let status = hub.status();
    let backends: Vec<_> = status
        .iter()
        .map(|bridge| bridge.backend.clone().unwrap_or_default())
        .collect();
    assert!(backends.contains(&"WASAPI".to_string()), "{backends:?}");
    assert!(backends.contains(&"CoreAudio".to_string()), "{backends:?}");
    let service = ZoneService::new(None, None);
    service.set_outputs(Arc::new(Outputs {
        bridges: Some(hub.clone()),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());

    let windows_zone = "bridge:study-pc-id:wasapi-0d0d0d0d111142228333444455556666";
    let mac_zone = "bridge:studio-mac-id:coreaudio-appleusbaudioengine-chord-qutest-1";
    let zones = service.list(USER, Some(&cache));
    for (id, rate, mode) in [
        (windows_zone, 384_000, "WASAPI exclusive mode"),
        (mac_zone, 384_000, "hog mode"),
    ] {
        let zone = zones.iter().find(|zone| zone.id == id).unwrap_or_else(|| {
            panic!(
                "{id} in {:?}",
                zones.iter().map(|zone| &zone.id).collect::<Vec<_>>()
            )
        });
        assert_eq!(zone.kind, ZoneKind::Bridge);
        assert_eq!(
            zone.output.as_ref().map(|output| output.max_sample_rate),
            Some(rate)
        );
        assert_eq!(zone.settings.volume_mode, VolumeMode::Hardware, "{id}");
        assert!(zone.settings.supports.exclusive);
        assert!(
            zone.notes
                .iter()
                .any(|note| note.contains(mode) && note.contains("not yet on real hardware")),
            "{:?}",
            zone.notes
        );
    }

    for zone_id in [windows_zone, mac_zone] {
        service
            .command(
                USER,
                zone_id,
                ZoneCommand::Load {
                    song_ids: ids(&["song-hires"]),
                    current_position: 0,
                    position_ms: 0,
                    play: true,
                },
                Some(&cache),
            )
            .await
            .expect("load");
        assert!(
            drive_until(&service, |service| zone(service, &cache, zone_id).status
                == ZoneStatus::Playing)
            .await
        );
        let stream = zone(&service, &cache, zone_id)
            .output
            .and_then(|output| output.stream)
            .expect("signal path");
        assert!(stream.bit_perfect, "{stream:?}");
        assert!(
            stream.output.starts_with("96 kHz/24-bit (S24_3LE)"),
            "{stream:?}"
        );
        assert!(
            drive_until(&service, |service| zone(service, &cache, zone_id).status
                == ZoneStatus::Stopped)
            .await,
            "plays to the end"
        );
    }

    // Windows: exclusive at exactly 96 kHz/24-bit, every sample unchanged.
    let capture = wasapi
        .api()
        .capture(&format!("{{0.0.0.00000000}}.{{{guid}}}"));
    let (format, mode, _) = capture.opened[0];
    assert_eq!(mode, parson_bridge::audio::wasapi::ShareMode::Exclusive);
    assert_eq!(
        (format.sample_rate, format.container_bits, format.valid_bits),
        (96_000, 24, 24)
    );
    assert!(
        capture.bytes.starts_with(&raw),
        "every sample reached the Windows DAC unchanged"
    );
    assert!(capture.bytes[raw.len()..].iter().all(|byte| *byte == 0));

    // macOS: hogged, switched to 96 kHz, 24-bit integer physical format,
    // and the DAC received every 24-bit sample exactly.
    let mac_capture = hal.api().capture(90);
    assert_eq!(mac_capture.runs[0], (24, 96_000.0));
    // Parson lets go of the DAC at the end of the queue (the stream closes
    // just after the zone reports it stopped).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while hal.api().hog_owner(90) != Ok(-1) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(hal.api().hog_owner(90), Ok(-1), "hog mode released after");
    let expected: Vec<i32> = raw
        .chunks(3)
        .map(|sample| i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8)
        .collect();
    let first = mac_capture
        .samples
        .iter()
        .position(|sample| *sample != 0)
        .unwrap_or(0)
        / 2
        * 2;
    assert!(
        mac_capture.samples[first..].starts_with(&expected),
        "every sample reached the Mac's DAC unchanged"
    );

    // Hardware volume in dB on both machines' DACs.
    for zone_id in [windows_zone, mac_zone] {
        service
            .command(
                USER,
                zone_id,
                ZoneCommand::SetVolume {
                    volume: 50,
                    member_id: None,
                    above_comfort: false,
                },
                None,
            )
            .await
            .expect("volume");
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        wasapi
            .api()
            .capture(&format!("{{0.0.0.00000000}}.{{{guid}}}"))
            .volume_db
            .is_some()
    );
    assert!(hal.api().capture(90).volume_db.is_some());
    let _ = std::fs::remove_dir_all(&directory);
}

/// A 16-bit 44.1 kHz FLAC made from exactly `raw` (s16le stereo).
fn cd_flac_from_raw(directory: &Path, name: &str, raw: &[u8]) -> PathBuf {
    let raw_path = directory.join(format!("{name}.s16"));
    std::fs::write(&raw_path, raw).expect("raw pcm");
    let path = directory.join(format!("{name}.flac"));
    run(std::process::Command::new("ffmpeg")
        .args([
            "-y", "-v", "error", "-f", "s16le", "-ar", "44100", "-ac", "2", "-i",
        ])
        .arg(&raw_path)
        .args(["-c:a", "flac", "-sample_fmt", "s16"])
        .arg(&path));
    path
}

#[actix_web::test]
async fn airplay_zones_stream_lossless_alac_to_a_receiver_end_to_end() {
    use crate::outputs::airplay::fake::FakeReceiver;
    use crate::outputs::airplay::{AirplayController, Receiver};
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-airplay-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    // 1.2 s of CD-quality noise, so every sample can be checked.
    let raw: Vec<u8> = raw_s24(52_920, 31)
        .chunks(3)
        .flat_map(|sample| [sample[1], sample[2]])
        .collect();
    let cd = cd_flac_from_raw(&directory, "cd", &raw);
    let hires = flac_from_raw(&directory, "hires", &raw_s24(96_000, 5), 96_000);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            song("song-cd", &cd, 1.2, "flac", 44_100, Some(16)),
            song("song-hires", &hires, 1.0, "flac", 96_000, Some(24)),
        ]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");

    let receiver = FakeReceiver::start(false);
    let airplay = AirplayController::new();
    airplay.add_receiver(Receiver {
        id: "aabbccddeeff".into(),
        name: "Kitchen speaker".into(),
        model: Some("AirPort10,115".into()),
        address: receiver.address.to_string(),
        supported: true,
        reason: None,
    });
    airplay.add_receiver(Receiver {
        id: "homepod".into(),
        name: "HomePod".into(),
        model: Some("AudioAccessory5,1".into()),
        address: "127.0.0.1:9".into(),
        supported: false,
        reason: Some("only AirPlay 2".into()),
    });
    let service = ZoneService::new(None, None);
    service.set_outputs(Arc::new(Outputs {
        airplay: Some(airplay.clone()),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());
    let id = "airplay:aabbccddeeff";
    let zones = service.list(USER, Some(&cache));
    assert!(
        !zones.iter().any(|zone| zone.name == "HomePod"),
        "receivers Parson cannot play to are not zones"
    );
    let speaker = zones
        .iter()
        .find(|zone| zone.id == id)
        .expect("AirPlay zone");
    assert_eq!(speaker.kind, ZoneKind::Airplay);
    assert_eq!(speaker.ecosystem, "airplay");
    assert_eq!(speaker.device, "AirPlay · AirPort10,115");
    assert!(!speaker.settings.supports.exclusive);
    assert!(speaker.notes[0].contains("44.1 kHz/16-bit lossless (ALAC)"));
    assert!(
        speaker
            .notes
            .iter()
            .any(|note| note.contains("not yet against Apple hardware"))
    );

    // --- CD-quality audio reaches the receiver bit for bit.
    service
        .command(
            USER,
            id,
            ZoneCommand::Load {
                song_ids: ids(&["song-cd"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("play on AirPlay");
    assert!(
        drive_until(&service, |service| zone(service, &cache, id).status
            == ZoneStatus::Playing)
        .await
    );
    let stream = zone(&service, &cache, id)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(stream.bit_perfect, "{stream:?}");
    assert_eq!(
        stream.output,
        "44.1 kHz/16-bit (S16_LE) to Kitchen speaker over AirPlay (ALAC)"
    );
    service
        .command(
            USER,
            id,
            ZoneCommand::SetVolume {
                volume: 50,
                member_id: None,
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("volume");
    assert!(
        drive_until(&service, |service| zone(service, &cache, id).status
            == ZoneStatus::Stopped)
        .await,
        "plays to the end"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let state = receiver.state();
    assert_eq!(
        &state.methods[..4],
        &["OPTIONS", "ANNOUNCE", "SETUP", "RECORD"]
    );
    assert!(
        state.methods.contains(&"TEARDOWN".to_string()),
        "{:?}",
        state.methods
    );
    assert!(state.sdp.contains("a=rtpmap:96 AppleLossless"));
    assert!(
        state
            .sdp
            .contains("a=fmtp:96 352 0 16 40 10 14 2 255 0 0 44100")
    );
    assert_eq!(state.bad_frames, 0);
    assert_eq!(state.markers, 1, "the first packet is marked");
    assert!(state.syncs >= 1);
    assert_eq!(state.sync_latency, Some(88_200));
    assert_eq!(
        state.timing_replies, 1,
        "answers the receiver's clock requests"
    );
    assert_eq!(state.resent, 1, "resends the packet the receiver lost");
    assert_eq!(state.volumes.last(), Some(&-15.0));
    let pcm = state.pcm();
    assert!(pcm.len() >= raw.len() && pcm.len() < raw.len() + 352 * 4);
    assert!(
        pcm[..raw.len()] == raw[..],
        "every sample arrives unchanged"
    );
    assert!(
        pcm[raw.len()..].iter().all(|byte| *byte == 0),
        "padded with silence"
    );

    // --- Hi-res is converted to what AirPlay carries, and says so.
    service
        .command(
            USER,
            id,
            ZoneCommand::Load {
                song_ids: ids(&["song-hires"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("hi-res on AirPlay");
    assert!(
        drive_until(&service, |service| zone(service, &cache, id).status
            == ZoneStatus::Playing)
        .await
    );
    let stream = zone(&service, &cache, id)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(!stream.bit_perfect);
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.contains("Resampled 96 kHz to 44.1 kHz"))
    );
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.contains("Reduced to 16-bit"))
    );
    service
        .command(USER, id, ZoneCommand::Stop, None)
        .await
        .expect("stop");
    let status = airplay.status();
    assert_eq!(status["receivers"].as_array().map(Vec::len), Some(2));
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn chromecast_zones_are_driven_by_the_server_end_to_end() {
    use crate::outputs::cast::fake::FakeCast;
    use crate::outputs::cast::{CastController, CastDevice};
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-cast-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let a = flac_from_raw(&directory, "a", &raw_s24(96_000 * 3 / 2, 41), 96_000);
    let b = tone(&directory, "b", "flac", 192_000, 1.0);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            song("song-a", &a, 1.5, "flac", 96_000, Some(24)),
            song("song-b", &b, 1.0, "flac", 192_000, Some(24)),
        ]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");
    let device = FakeCast::start();
    let cast = CastController::new();
    cast.add(CastDevice {
        id: "living-tv".into(),
        name: "Living Room TV".into(),
        model: "Chromecast Ultra".into(),
        address: device.address.to_string(),
        group: false,
    });
    cast.add(CastDevice {
        id: "whole-home".into(),
        name: "Whole Home".into(),
        model: "Google Cast Group".into(),
        address: device.address.to_string(),
        group: true,
    });
    let service = web::Data::new(ZoneService::new(None, None));
    service.set_outputs(Arc::new(Outputs {
        cast: Some(cast.clone()),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());
    let (port, server) =
        serve_device_routes(service.clone(), web::Data::from(lifecycle.clone())).await;
    service.set_port(port);

    let id = "chromecast:living-tv";
    let listed = zone(&service, &cache, id);
    assert_eq!(listed.kind, ZoneKind::Chromecast);
    assert_eq!(listed.device, "Chromecast Ultra");
    assert!(
        listed.capabilities.transfer_target,
        "any app can transfer to it"
    );
    let output = listed.output.clone().expect("output");
    assert_eq!((output.max_sample_rate, output.max_bit_depth), (96_000, 24));
    assert!(
        listed
            .notes
            .iter()
            .any(|note| note.contains("not yet on Google hardware"))
    );
    assert_eq!(
        zone(&service, &cache, "chromecast:whole-home").device,
        "Google Home speaker group"
    );

    // --- Load from the server: launch the receiver app, LOAD signed media.
    service
        .command(
            USER,
            id,
            ZoneCommand::Load {
                song_ids: ids(&["song-a", "song-b"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("cast from the server");
    assert!(
        drive_until(&service, |service| zone(service, &cache, id).status
            == ZoneStatus::Playing)
        .await
    );
    let stream = zone(&service, &cache, id)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(stream.bit_perfect, "{stream:?}");
    let state = device.state();
    assert!(state.launched);
    assert_eq!(state.launches, 1);
    assert_eq!(
        state.connects,
        vec!["receiver-0".to_string(), "web-5".to_string()]
    );
    let load = &state.loads[0];
    assert_eq!(load["media"]["contentType"], "audio/flac");
    assert_eq!(load["media"]["metadata"]["title"], "Track song-a");
    assert_eq!(load["media"]["streamType"], "BUFFERED");

    // --- Volume on the device's own scale, then pause and resume.
    service
        .command(
            USER,
            id,
            ZoneCommand::SetVolume {
                volume: 30,
                member_id: None,
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("volume");
    assert!((device.state().level - 0.3).abs() < 1e-9);
    assert_eq!(
        zone(&service, &cache, id).volume.map(|volume| volume.level),
        Some(30)
    );
    service
        .command(USER, id, ZoneCommand::Pause, None)
        .await
        .expect("pause");
    assert!(
        drive_until(&service, |service| zone(service, &cache, id).status
            == ZoneStatus::Paused)
        .await
    );
    service
        .command(USER, id, ZoneCommand::Play, None)
        .await
        .expect("resume");

    // --- The device fetched the original 96 kHz FLAC; at the end of the
    // track the server loads the next one, converted from 192 kHz.
    assert!(
        drive_until(&service, |service| zone(service, &cache, id)
            .queue
            .current_position
            == 1)
        .await,
        "moves on when the device finishes a track"
    );
    assert!(
        drive_until(&service, |_| device.state().fetches.len() >= 2).await,
        "fetches both tracks"
    );
    let state = device.state();
    assert_eq!(state.loads.len(), 2);
    assert!(state.commands.contains(&"PAUSE".to_string()));
    for (index, expected_rate) in [(0, 96_000), (1, 96_000)] {
        let fetch = &state.fetches[index];
        assert_eq!(fetch.status, 200, "{}", fetch.url);
        assert_eq!(fetch.content_type, "audio/flac");
        assert!(fetch.body.starts_with(b"fLaC"));
        let path = directory.join(format!("fetched-{index}.flac"));
        std::fs::write(&path, &fetch.body).expect("fetched media");
        assert_eq!(ffprobe(&path), (expected_rate, "flac".to_string()));
    }
    let stream = zone(&service, &cache, id)
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert!(!stream.bit_perfect);
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.contains("192 kHz to 96 kHz")),
        "{stream:?}"
    );
    service
        .command(USER, id, ZoneCommand::Stop, None)
        .await
        .expect("stop");
    server.stop(true).await;
    let _ = std::fs::remove_dir_all(&directory);
}

/// Ungrouping a bridge group mid-track: the first zone carries on from the
/// group's position, plays to the end, then takes stop and new loads (the
/// round-5 bug left it frozen at "playing" with the old file).
#[actix_web::test]
async fn bridge_zone_carries_on_after_ungrouping_mid_track_and_stays_controllable() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-ungroup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let raw_long = raw_s24(44_100 * 3, 31);
    let raw_hi = raw_s24(96_000, 37);
    let long = flac_from_raw(&directory, "long", &raw_long, 44_100);
    let hi = flac_from_raw(&directory, "hi", &raw_hi, 96_000);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![
            song("song-long", &long, 3.0, "flac", 44_100, Some(24)),
            song("song-hi", &hi, 1.0, "flac", 96_000, Some(24)),
        ]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");
    let (kitchen, _kitchen_dacs) = bridge_endpoint("kitchen");
    let (living, living_dacs) = bridge_endpoint("living-room");
    let hub = crate::outputs::bridge::BridgeHub::new("server-id".into(), "Test".into(), None);
    hub.add(&kitchen.address().to_string()).expect("kitchen");
    hub.add(&living.address().to_string()).expect("living room");
    let service = ZoneService::new(None, None);
    service.set_outputs(Arc::new(Outputs {
        bridges: Some(hub.clone()),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());
    let kitchen_dac = "bridge:kitchen-id:sim-usb-dac";
    let living_dac = "bridge:living-room-id:sim-usb-dac";

    // The living room plays a two-track queue when it joins the kitchen.
    service
        .command(
            USER,
            living_dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-long", "song-hi"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load the living room");
    assert!(
        drive_until(&service, |service| zone(service, &cache, living_dac).status
            == ZoneStatus::Playing)
        .await
    );
    let group_id = service
        .group(USER, &ids(&[living_dac, kitchen_dac]))
        .await
        .expect("group");
    assert!(
        drive_until(&service, |service| {
            let group = zone(service, &cache, &group_id);
            group.status == ZoneStatus::Playing && group.position_ms > 600
        })
        .await,
        "the group plays"
    );

    // Ungroup while it plays: the first zone carries on.
    service
        .ungroup(USER, &group_id, None)
        .await
        .expect("ungroup mid-track");
    assert!(
        drive_until(&service, |service| zone(service, &cache, living_dac).status
            == ZoneStatus::Playing)
        .await,
        "the first zone carries on"
    );
    assert!(
        drive_until(&service, |service| {
            let living = zone(service, &cache, living_dac);
            living.queue.current_position == 1
                && living.status == ZoneStatus::Playing
                && living
                    .output
                    .as_ref()
                    .and_then(|output| output.stream.as_ref())
                    .is_some_and(|stream| stream.output.starts_with("96 kHz/24-bit"))
        })
        .await,
        "it moves on to the next track: {:?}",
        zone(&service, &cache, living_dac)
    );
    assert!(
        drive_until(&service, |service| zone(service, &cache, living_dac).status
            == ZoneStatus::Stopped)
        .await,
        "and plays its queue to the end instead of freezing: {:?}",
        zone(&service, &cache, living_dac)
    );

    // A new load plays the new file (signal path included), and stop works.
    service
        .command(
            USER,
            living_dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-hi", "song-long"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load after the ungroup");
    assert!(
        drive_until(&service, |service| {
            let living = zone(service, &cache, living_dac);
            living.status == ZoneStatus::Playing
                && living
                    .output
                    .as_ref()
                    .and_then(|output| output.stream.as_ref())
                    .is_some_and(|stream| stream.output.starts_with("96 kHz/24-bit"))
        })
        .await,
        "the new file plays: {:?}",
        zone(&service, &cache, living_dac)
    );
    service
        .command(USER, living_dac, ZoneCommand::Stop, Some(&cache))
        .await
        .expect("stop");
    assert!(
        drive_until(&service, |service| zone(service, &cache, living_dac).status
            == ZoneStatus::Stopped)
        .await,
        "stop works after the ungroup"
    );
    // The second zone is free and plays on its own too.
    service
        .command(
            USER,
            kitchen_dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-hi"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load the kitchen");
    assert!(
        drive_until(&service, |service| zone(service, &cache, kitchen_dac)
            .status
            == ZoneStatus::Stopped)
        .await,
        "the kitchen plays its own file to the end"
    );
    assert!(
        living_dacs
            .capture("sim-usb-dac")
            .opened
            .iter()
            .any(|(format, _)| format.sample_rate == 96_000),
        "the living room DAC reopened at the new file's rate"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn audio_device_standby_releases_the_dac_and_play_wakes_it_where_it_stopped() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let directory = std::env::temp_dir().join(format!("parson-standby-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let long = tone(&directory, "long", "flac", 44_100, 20.0);
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle
        .set_ready(library(vec![song(
            "song-long",
            &long,
            20.0,
            "flac",
            44_100,
            Some(16),
        )]))
        .await;
    let cache = lifecycle.cache().await.expect("cache");
    // Real time, so the track is still playing when standby comes.
    let backend = Arc::new(SimulatedBackend::with_default_devices(1.0));
    let devices = Arc::new(DeviceManager::new(backend.clone(), None));
    let service = ZoneService::new(None, None);
    service.set_outputs(Arc::new(Outputs {
        upnp: None,
        devices: Some(devices),
        ..Outputs::default()
    }));
    service.set_library(lifecycle.clone());
    let dac = "device:sim-usb-dac";
    assert!(zone(&service, &cache, dac).capabilities.standby);

    service
        .command(
            USER,
            dac,
            ZoneCommand::Load {
                song_ids: ids(&["song-long"]),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("play");
    assert!(
        drive_until(&service, |service| {
            let zone = zone(service, &cache, dac);
            zone.status == ZoneStatus::Playing && zone.position_ms >= 500
        })
        .await
    );
    service
        .command(USER, dac, ZoneCommand::Standby, Some(&cache))
        .await
        .expect("standby");
    assert!(
        drive_until(&service, |service| zone(service, &cache, dac).status
            == ZoneStatus::Standby)
        .await
    );
    // The device thread closes the stream right after; then nothing flows.
    let mut parked = backend.capture("sim-usb-dac").bytes.len();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let now = backend.capture("sim-usb-dac").bytes.len();
        if now == parked {
            break;
        }
        parked = now;
    }
    for tick in 0..10 {
        service.drive_hosted(10_000 + tick).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let standby = zone(&service, &cache, dac);
    assert_eq!(standby.status, ZoneStatus::Standby, "stays off");
    assert_eq!(standby.queue.song_ids, ids(&["song-long"]), "queue kept");
    assert_eq!(
        backend.capture("sim-usb-dac").bytes.len(),
        parked,
        "the DAC gets nothing in standby"
    );

    // Wake leaves standby without playing; play resumes the track.
    service
        .command(USER, dac, ZoneCommand::Wake, Some(&cache))
        .await
        .expect("wake");
    assert_ne!(zone(&service, &cache, dac).status, ZoneStatus::Standby);
    assert_ne!(zone(&service, &cache, dac).status, ZoneStatus::Playing);
    service
        .command(USER, dac, ZoneCommand::Standby, Some(&cache))
        .await
        .expect("standby again");
    service
        .command(USER, dac, ZoneCommand::Play, Some(&cache))
        .await
        .expect("play wakes the zone");
    assert!(
        drive_until(&service, |service| {
            let zone = zone(service, &cache, dac);
            zone.status == ZoneStatus::Playing && zone.position_ms >= 500
        })
        .await,
        "plays again from where standby left it"
    );
    assert!(backend.capture("sim-usb-dac").bytes.len() > parked);

    // Shared zones: Home Assistant volume requests go to every account.
    service
        .update_settings(
            USER,
            dac,
            ZoneSettingsPatch {
                volume_control: Some(VolumeControl::HomeAssistant),
                ..ZoneSettingsPatch::default()
            },
            Viewer::default(),
            Some(&cache),
        )
        .await
        .expect("hand the volume to Home Assistant");
    let mut events = service.subscribe();
    service
        .command(
            USER,
            dac,
            ZoneCommand::AdjustVolume {
                delta: -2,
                member_id: None,
            },
            Some(&cache),
        )
        .await
        .expect("volume down");
    match events.try_recv() {
        Ok(ZoneEvent::VolumeRequest { user_id, request }) => {
            assert_eq!(user_id, None);
            assert_eq!(
                (request.zone_id.as_str(), request.action),
                (dac, "volume_down")
            );
        }
        other => panic!("expected a volume request, got {other:?}"),
    }
    assert_eq!(
        backend.capture("sim-usb-dac").volume_db,
        None,
        "the DAC's own volume stays untouched"
    );
    std::fs::remove_dir_all(&directory).ok();
}
