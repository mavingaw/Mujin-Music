//! End-to-end zone tests against the fake Sonos household: SSDP discovery,
//! topology with a stereo pair, transfer from a web client onto Sonos with
//! signed media URLs the "speaker" actually streams, UPnP eventing, volume
//! limits, native grouping, linked groups, external playback and the HTTP
//! API.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::dev::Service as _;
use actix_web::test as actix_test;
use actix_web::{App, HttpMessage, HttpServer, web};

use super::service::ClientRegistration;
use super::*;
use crate::api::auth::Claims;
use crate::domain::{Album, Artist, AudioProperties, Song};
use crate::library::search::SearchIndex;
use crate::library::state::{LibraryCache, LibraryLifecycle};
use crate::sonos::fake::{FakePlayerSpec, FakeSonos};
use crate::sonos::{SonosConfig, SonosController};

const LIVING: &str = "RINCON_LIVING01400";
const LIVING_PAIR: &str = "RINCON_LIVINGR01400";
const KITCHEN: &str = "RINCON_KITCHEN01400";
const USER: i32 = 7;

fn library(directory: &std::path::Path) -> LibraryCache {
    let mut songs = Vec::new();
    for (index, (id, name)) in [
        ("song-1", "First & Foremost"),
        ("song-2", "Second <Song>"),
        ("song-3", "Third"),
        ("song-4", "Not Queued"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = directory.join(format!("{id}.mp3"));
        std::fs::write(&path, vec![0x49_u8; 4_096 + index]).expect("fixture audio");
        songs.push(Song {
            id: id.to_string(),
            name: name.to_string(),
            artist: "Fixture Artist".to_string(),
            track_number: index as u16 + 1,
            path: path.to_string_lossy().to_string(),
            duration: 180.0,
            audio: Some(AudioProperties {
                codec: Some("mp3".to_string()),
                container: Some("mp3".to_string()),
                sample_rate: Some(44_100),
                bit_depth: None,
                channels: Some(2),
                bitrate_kbps: Some(320),
                lossless: Some(false),
            }),
            ..Song::default()
        });
    }
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
        songs_by_artist: HashMap::from([("artist-1".to_string(), vec![0, 1, 2, 3])]),
        songs_by_genre: HashMap::new(),
        image_paths: Default::default(),
    }
}

fn web_client(status: ZoneStatus, song_ids: Option<Vec<String>>) -> ClientRegistration {
    ClientRegistration {
        kind: ZoneKind::Web,
        name: "Office browser".to_string(),
        device: "Chrome on Linux".to_string(),
        status,
        song_ids,
        current_position: 1,
        position_ms: 30_000,
        duration_ms: 180_000,
        volume: 80,
        muted: false,
        acknowledged_seq: 0,
    }
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

fn test_pool() -> crate::persistence::connection::DbPool {
    use diesel::r2d2::{ConnectionManager, Pool};
    Arc::new(
        Pool::builder()
            .max_size(1)
            .build(ConnectionManager::<diesel::sqlite::SqliteConnection>::new(
                ":memory:",
            ))
            .expect("test pool"),
    )
}

async fn wait_until<F: Fn() -> bool>(condition: F) -> bool {
    for _ in 0..250 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[actix_web::test]
async fn sonos_zones_play_transfer_group_and_follow_events_end_to_end() {
    let fake = FakeSonos::start(&[
        FakePlayerSpec {
            uuid: LIVING,
            room: "Living Room",
            model: "Sonos Play:5",
            pair_of: None,
        },
        FakePlayerSpec {
            uuid: LIVING_PAIR,
            room: "Living Room",
            model: "Sonos Play:5",
            pair_of: Some(LIVING),
        },
        FakePlayerSpec {
            uuid: KITCHEN,
            room: "Kitchen",
            model: "Sonos One",
            pair_of: None,
        },
    ])
    .await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle.set_ready(library(&directory)).await;
    let cache = lifecycle.cache().await.expect("cache");

    let sonos = Arc::new(
        SonosController::new(SonosConfig {
            ssdp_targets: vec![fake.ssdp_address],
            static_hosts: Vec::new(),
            search_timeout: Duration::from_millis(400),
        })
        .expect("controller"),
    );
    let service = web::Data::new(ZoneService::new(Some(sonos.clone()), None));
    let lifecycle_data = web::Data::from(lifecycle.clone());

    // Real HTTP server for the routes Sonos players call.
    let server_service = service.clone();
    let server_lifecycle = lifecycle_data.clone();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(server_service.clone())
            .app_data(server_lifecycle.clone())
            .configure(crate::api::zones::configure_device_routes)
    })
    .workers(1)
    .bind(("127.0.0.1", 0))
    .expect("bind test server");
    let port = server.addrs()[0].port();
    let server = server.run();
    let server_handle = server.handle();
    actix_web::rt::spawn(server);
    service.set_port(port);

    // --- SSDP discovery + topology: the stereo pair is one room.
    assert_eq!(sonos.discover().await.expect("discovery"), 3);
    sonos.ensure_subscriptions(port).await;
    let zones = service.list(USER, Some(&cache));
    let sonos_zones: Vec<&Zone> = zones
        .iter()
        .filter(|zone| zone.kind == ZoneKind::Sonos)
        .collect();
    assert_eq!(sonos_zones.len(), 2, "stereo pair secondary is hidden");
    let living_id = format!("sonos:{LIVING}");
    let kitchen_id = format!("sonos:{KITCHEN}");
    let living = zones
        .iter()
        .find(|zone| zone.id == living_id)
        .expect("living");
    assert_eq!(living.name, "Living Room");
    assert_eq!(living.device, "Sonos Play:5");
    assert_eq!(living.status, ZoneStatus::Idle);
    assert!(
        living
            .notes
            .iter()
            .any(|note| note.contains("48 kHz/24-bit"))
    );

    // --- A web client registers as a zone with a queue.
    let queue: Vec<String> = ["song-1", "song-2", "song-3"].map(String::from).to_vec();
    let registered = service
        .register_client(
            USER,
            "browser-0001",
            web_client(ZoneStatus::Playing, Some(queue.clone())),
        )
        .expect("register");
    assert_eq!(registered.zone_id, "client:browser-0001");
    assert!(!registered.needs_queue);
    let other_user = service.list(USER + 1, Some(&cache));
    assert!(
        other_user
            .iter()
            .all(|zone| zone.id != "client:browser-0001")
    );
    assert_eq!(
        other_user
            .iter()
            .filter(|zone| zone.kind == ZoneKind::Sonos)
            .count(),
        2
    );

    // --- Transfer queue + position from the browser to the Living Room.
    service
        .transfer(USER, "client:browser-0001", &living_id, Some(&cache))
        .await
        .expect("transfer");
    let player = fake.player(LIVING);
    assert_eq!(player.queue.len(), 3);
    assert_eq!(player.transport_uri, format!("x-rincon-queue:{LIVING}#0"));
    assert_eq!(player.track, 2, "transfer keeps the queue position");
    assert_eq!(player.state, "PLAYING");
    assert_eq!(
        player.position_ms / 1_000,
        30,
        "transfer keeps the track position"
    );
    assert!(
        player.queue[1]
            .1
            .contains("<dc:title>Second &lt;Song&gt;</dc:title>")
    );
    assert!(player.queue[1].1.contains("http-get:*:audio/mpeg:*"));
    let uri = &player.queue[1].0;
    assert!(uri.starts_with(&format!(
        "http://127.0.0.1:{port}/api/v1/zones/media/song-2/stream?zone={LIVING}&expires="
    )));
    assert_eq!(fake.calls("RemoveAllTracksFromQueue").len(), 1);
    assert!(
        fake.calls("Seek")
            .iter()
            .any(|call| call.arguments["Unit"] == "REL_TIME"
                && call.arguments["Target"] == "0:00:30")
    );
    // The source browser was told to pause.
    let pending = service
        .register_client(USER, "browser-0001", web_client(ZoneStatus::Playing, None))
        .expect("heartbeat");
    assert!(
        pending
            .commands
            .iter()
            .any(|command| command.command == ZoneCommand::Pause)
    );

    // --- The speaker fetches the signed URL and gets audio.
    assert!(
        fake.wait_for(|household| !household.media_fetches.is_empty())
            .await
    );
    let fetch = fake.media_fetches()[0].clone();
    assert_eq!(fetch.status, 200, "speaker could not stream {}", fetch.uri);
    assert_eq!(fetch.content_type.as_deref(), Some("audio/mpeg"));
    assert_eq!(fetch.bytes, 4_097);

    // Tampered or out-of-queue URLs are refused.
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    let tampered = uri.replace("signature=", "signature=AAAA");
    assert_eq!(
        http.get(&tampered).send().await.expect("tampered").status(),
        401
    );
    let expires = chrono::Utc::now().timestamp() + 60;
    let foreign = format!(
        "http://127.0.0.1:{port}/api/v1/zones/media/song-4/stream?zone={LIVING}&expires={expires}&signature={}",
        crate::api::cast::sign_media_url(&format!("zone:{LIVING}"), "song-4", expires)
    );
    assert_eq!(
        http.get(&foreign).send().await.expect("foreign").status(),
        401
    );

    // --- Polling reflects state; now playing comes from the Parson queue.
    sonos.poll_all().await;
    let living = service
        .zone(USER, &living_id, Some(&cache))
        .expect("living");
    assert_eq!(living.status, ZoneStatus::Playing);
    assert_eq!(living.source, ZoneSource::Parson);
    assert_eq!(living.queue.current_position, 1);
    assert_eq!(
        living.now_playing.as_ref().map(|now| now.title.as_str()),
        Some("Second <Song>")
    );

    // --- UPnP events: the track ends on the speaker, NOTIFY reaches us.
    fake.advance(LIVING);
    assert!(
        wait_until(|| service
            .zone(USER, &living_id, Some(&cache))
            .is_some_and(|zone| zone.queue.current_position == 2))
        .await,
        "NOTIFY did not refresh the zone"
    );
    assert!(fake.notifications_sent() >= 1);
    assert_eq!(
        service
            .command(USER, &living_id, ZoneCommand::Next, Some(&cache))
            .await,
        Err(ZoneError::Invalid("end_of_queue"))
    );
    service
        .command(
            USER,
            &living_id,
            ZoneCommand::Jump { queue_position: 0 },
            Some(&cache),
        )
        .await
        .expect("jump");
    assert_eq!(fake.player(LIVING).track, 1);

    // --- Per-zone volume, mute and the safety limit.
    service
        .command(
            USER,
            &living_id,
            ZoneCommand::SetVolume {
                volume: 35,
                member_id: None,
            },
            None,
        )
        .await
        .expect("volume");
    assert_eq!(fake.player(LIVING).volume, 35);
    service
        .update_settings(
            USER,
            &living_id,
            ZoneSettings {
                name: Some("Lounge".to_string()),
                max_volume: Some(40),
            },
        )
        .await
        .expect("settings");
    service
        .command(
            USER,
            &living_id,
            ZoneCommand::SetVolume {
                volume: 90,
                member_id: None,
            },
            None,
        )
        .await
        .expect("limited volume");
    assert_eq!(fake.player(LIVING).volume, 40, "safety limit applies");
    service
        .command(
            USER,
            &living_id,
            ZoneCommand::SetMute {
                muted: true,
                member_id: None,
            },
            None,
        )
        .await
        .expect("mute");
    assert!(fake.player(LIVING).muted);
    let living = service.zone(USER, &living_id, None).expect("living");
    assert_eq!(living.name, "Lounge");
    assert_eq!(
        living
            .volume
            .as_ref()
            .map(|volume| (volume.level, volume.muted, volume.max)),
        Some((40, true, 40))
    );

    // --- Native Sonos grouping (sample-synced) and per-room volume.
    let group_id = service
        .group(USER, &[living_id.clone(), kitchen_id.clone()])
        .await
        .expect("group");
    assert_eq!(group_id, living_id);
    assert_eq!(fake.player(KITCHEN).coordinator, LIVING);
    let join = &fake.calls("SetAVTransportURI");
    assert!(join.iter().any(|call| call.player == KITCHEN
        && call.arguments["CurrentURI"] == format!("x-rincon:{LIVING}")));
    let zones = service.list(USER, Some(&cache));
    let sonos_zones: Vec<&Zone> = zones
        .iter()
        .filter(|zone| zone.kind == ZoneKind::Sonos)
        .collect();
    assert_eq!(sonos_zones.len(), 1);
    let group = sonos_zones[0].group.as_ref().expect("group info");
    assert!(group.synced);
    assert_eq!(group.members.len(), 2);
    service
        .update_settings(USER, &living_id, ZoneSettings::default())
        .await
        .expect("clear settings");
    assert_eq!(
        service.zone(USER, &living_id, None).expect("group").name,
        "Living Room + Kitchen"
    );
    service
        .command(
            USER,
            &living_id,
            ZoneCommand::SetVolume {
                volume: 12,
                member_id: Some(kitchen_id.clone()),
            },
            None,
        )
        .await
        .expect("member volume");
    assert_eq!(fake.player(KITCHEN).volume, 12);
    assert_eq!(fake.player(LIVING).volume, 40);
    service
        .ungroup(USER, &living_id, None)
        .await
        .expect("ungroup");
    assert_eq!(fake.player(KITCHEN).coordinator, KITCHEN);
    assert_eq!(
        service
            .list(USER, None)
            .iter()
            .filter(|zone| zone.kind == ZoneKind::Sonos)
            .count(),
        2
    );

    // --- Cross-ecosystem groups are linked and say they are not synced.
    let linked = service
        .group(
            USER,
            &["client:browser-0001".to_string(), kitchen_id.clone()],
        )
        .await
        .expect("linked group");
    assert!(linked.starts_with("linked:"));
    let zone = service
        .zone(USER, &linked, Some(&cache))
        .expect("linked zone");
    let info = zone.group.expect("linked info");
    assert!(!info.synced);
    assert!(info.sync_note.contains("not sample-synced"));
    assert_eq!(
        service
            .zone(USER, &kitchen_id, None)
            .and_then(|zone| zone.grouped_into),
        Some(linked.clone())
    );
    let pauses_before = fake.calls("Pause").len();
    service
        .command(USER, &linked, ZoneCommand::Pause, None)
        .await
        .expect("linked pause");
    assert_eq!(fake.calls("Pause").len(), pauses_before + 1);
    let pending = service
        .register_client(USER, "browser-0001", web_client(ZoneStatus::Paused, None))
        .expect("heartbeat");
    assert_eq!(
        pending
            .commands
            .last()
            .map(|command| command.command.clone()),
        Some(ZoneCommand::Pause)
    );
    service.ungroup(USER, &linked, None).await.expect("unlink");
    assert!(service.zone(USER, &linked, None).is_none());

    // --- Something the Sonos app started shows as external playback.
    fake.play_external(KITCHEN, "x-sonos-http:radio.mp3");
    sonos.poll_all().await;
    let kitchen = service.zone(USER, &kitchen_id, None).expect("kitchen");
    assert_eq!(kitchen.source, ZoneSource::External);
    assert!(!kitchen.capabilities.queue);

    // --- Transfer back from Sonos to the browser carries the queue.
    service
        .transfer(USER, &living_id, "client:browser-0001", Some(&cache))
        .await
        .expect("transfer back");
    let pending = service
        .register_client(USER, "browser-0001", web_client(ZoneStatus::Paused, None))
        .expect("heartbeat");
    let load = pending
        .commands
        .iter()
        .find_map(|command| match &command.command {
            ZoneCommand::Load {
                song_ids,
                current_position,
                ..
            } => Some((song_ids.clone(), *current_position)),
            _ => None,
        })
        .expect("load command for the browser");
    assert_eq!(load, (queue.clone(), 0));
    assert_eq!(
        fake.player(LIVING).state,
        "STOPPED",
        "source stopped after transfer"
    );

    server_handle.stop(false).await;
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn zone_http_api_lists_controls_and_validates() {
    let service = web::Data::new(ZoneService::new(None, None));
    let lifecycle = web::Data::new(LibraryLifecycle::new());
    let pool = web::Data::new(test_pool());
    let app = actix_test::init_service(
        App::new()
            .app_data(service.clone())
            .app_data(lifecycle.clone())
            .app_data(pool.clone())
            .wrap_fn(|request, next| {
                request.extensions_mut().insert(claims());
                next.call(request)
            })
            .configure(crate::api::zones::configure),
    )
    .await;

    let registered: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::put()
            .uri("/zones/clients/desktop-abcdef")
            .set_json(serde_json::json!({
                "kind": "desktop",
                "name": "Studio PC",
                "device": "Parson for Windows",
                "status": "playing",
                "song_ids": ["song-1"],
                "current_position": 0,
                "position_ms": 1000,
                "duration_ms": 60000,
                "volume": 50,
                "muted": false
            }))
            .to_request(),
    )
    .await;
    assert_eq!(registered["zone_id"], "client:desktop-abcdef");

    let listed: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get().uri("/zones").to_request(),
    )
    .await;
    assert_eq!(listed["zones"][0]["kind"], "desktop");
    assert_eq!(listed["zones"][0]["name"], "Studio PC");
    assert_eq!(listed["zones"][0]["volume"]["level"], 50);
    assert_eq!(listed["sonos"]["enabled"], false);

    let accepted = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri("/zones/client:desktop-abcdef/commands")
            .set_json(serde_json::json!({"command": "adjust_volume", "delta": 10}))
            .to_request(),
    )
    .await;
    assert_eq!(accepted.status(), 202);
    let pending = service
        .register_client(
            USER,
            "desktop-abcdef",
            ClientRegistration {
                kind: ZoneKind::Desktop,
                name: "Studio PC".into(),
                device: String::new(),
                status: ZoneStatus::Playing,
                song_ids: None,
                current_position: 0,
                position_ms: 2_000,
                duration_ms: 60_000,
                volume: 50,
                muted: false,
                acknowledged_seq: 0,
            },
        )
        .expect("heartbeat");
    assert_eq!(
        pending.commands[0].command,
        ZoneCommand::SetVolume {
            volume: 60,
            member_id: None
        }
    );

    for (uri, body, status) in [
        (
            "/zones/client:desktop-abcdef/commands",
            serde_json::json!({"command": "seek", "position_ms": -1}),
            400,
        ),
        (
            "/zones/client:desktop-abcdef/commands",
            serde_json::json!({"command": "load", "song_ids": []}),
            400,
        ),
        (
            "/zones/client:nobody-123456/commands",
            serde_json::json!({"command": "play"}),
            404,
        ),
        (
            "/zones/groups",
            serde_json::json!({"zone_ids": ["client:desktop-abcdef"]}),
            400,
        ),
        (
            "/zones/groups",
            serde_json::json!({"zone_ids": ["client:desktop-abcdef", "cast:abc"]}),
            409,
        ),
        (
            "/zones/client:desktop-abcdef/sleep",
            serde_json::json!({"minutes": 0}),
            400,
        ),
    ] {
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri(uri)
                .set_json(body)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status, "{uri}");
    }

    let sleep = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri("/zones/client:desktop-abcdef/sleep")
            .set_json(serde_json::json!({"minutes": 30}))
            .to_request(),
    )
    .await;
    assert_eq!(sleep.status(), 204);
    let listed: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get().uri("/zones").to_request(),
    )
    .await;
    assert!(
        listed["zones"][0]["sleep_timer"]["ends_at_ms"]
            .as_i64()
            .is_some()
    );

    let deleted = actix_test::call_service(
        &app,
        actix_test::TestRequest::delete()
            .uri("/zones/clients/desktop-abcdef")
            .to_request(),
    )
    .await;
    assert_eq!(deleted.status(), 204);
    assert!(service.list(USER, None).is_empty());
}

#[test]
fn sleep_timer_lets_a_nearly_finished_track_end() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let service = ZoneService::new(None, None);
        let mut report = web_client(ZoneStatus::Playing, Some(vec!["song-1".into()]));
        report.current_position = 0;
        report.position_ms = 170_000;
        report.duration_ms = 180_000;
        service
            .register_client(USER, "browser-sleep1", report)
            .expect("register");
        service
            .set_sleep_timer(USER, "client:browser-sleep1", Some(1))
            .expect("timer");
        // Force the timer due now.
        service.force_sleep_due("client:browser-sleep1");
        service.run_sleep_timers(|_| None).await;
        let zone = service
            .zone(USER, "client:browser-sleep1", None)
            .expect("zone");
        let timer = zone.sleep_timer.expect("timer kept while the track ends");
        assert!(timer.finishing_track);
        service.force_sleep_due("client:browser-sleep1");
        service.run_sleep_timers(|_| None).await;
        let zone = service
            .zone(USER, "client:browser-sleep1", None)
            .expect("zone");
        assert!(zone.sleep_timer.is_none());
        let mut heartbeat = web_client(ZoneStatus::Playing, None);
        heartbeat.current_position = 0;
        let pending = service
            .register_client(USER, "browser-sleep1", heartbeat)
            .expect("heartbeat");
        assert_eq!(
            pending
                .commands
                .last()
                .map(|command| command.command.clone()),
            Some(ZoneCommand::Pause)
        );
    });
}

#[test]
fn sonos_stream_plan_keeps_native_formats_and_converts_hi_res() {
    let song = |path: &str, codec: &str, rate: u32, depth: Option<u8>| Song {
        path: path.to_string(),
        audio: Some(AudioProperties {
            codec: Some(codec.to_string()),
            sample_rate: Some(rate),
            bit_depth: depth,
            ..AudioProperties::default()
        }),
        ..Song::default()
    };
    let plan = crate::api::library::sonos_stream_plan;
    assert_eq!(plan(&song("a.mp3", "mp3", 44_100, None)).convert_to, None);
    assert_eq!(
        plan(&song("a.flac", "flac", 48_000, Some(24))).mime_type,
        "audio/flac"
    );
    assert_eq!(
        plan(&song("a.flac", "flac", 48_000, Some(24))).convert_to,
        None
    );
    assert_eq!(
        plan(&song("a.flac", "flac", 96_000, Some(24))).convert_to,
        Some((48_000, 24))
    );
    assert_eq!(
        plan(&song("a.flac", "flac", 88_200, Some(24))).convert_to,
        Some((44_100, 24))
    );
    assert_eq!(
        plan(&song("a.m4a", "alac", 192_000, Some(32))).convert_to,
        Some((48_000, 24))
    );
    assert_eq!(
        plan(&song("a.m4a", "aac", 44_100, None)).mime_type,
        "audio/mp4"
    );
    assert_eq!(
        plan(&song("a.ogg", "opus", 48_000, None)).convert_to,
        Some((48_000, 16))
    );
}

/// Manual helper: runs the fake household until killed and prints the
/// player addresses for `PARSON_SONOS_HOSTS`.
#[actix_web::test]
#[ignore = "manual: serves a fake Sonos household until interrupted"]
async fn serve_fake_sonos_household() {
    let fake = FakeSonos::start(&[
        FakePlayerSpec {
            uuid: LIVING,
            room: "Living Room",
            model: "Sonos Five",
            pair_of: None,
        },
        FakePlayerSpec {
            uuid: LIVING_PAIR,
            room: "Living Room",
            model: "Sonos Five",
            pair_of: Some(LIVING),
        },
        FakePlayerSpec {
            uuid: KITCHEN,
            room: "Kitchen",
            model: "Sonos One",
            pair_of: None,
        },
        FakePlayerSpec {
            uuid: "RINCON_BEDROOM01400",
            room: "Bedroom",
            model: "Sonos Era 100",
            pair_of: None,
        },
    ])
    .await;
    let hosts: Vec<String> = fake
        .household
        .lock()
        .unwrap()
        .players
        .iter()
        .map(|player| player.address.to_string())
        .collect();
    println!("PARSON_SONOS_HOSTS={}", hosts.join(","));
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
