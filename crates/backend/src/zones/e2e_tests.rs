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
        repeat: RepeatMode::Off,
        shuffle: false,
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
                above_comfort: false,
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
            ZoneSettingsPatch {
                name: Some("Lounge".to_string()),
                max_volume: Some(Some(40)),
                ..ZoneSettingsPatch::default()
            },
            super::Viewer::default(),
            None,
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
                above_comfort: false,
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
        .update_settings(
            USER,
            &living_id,
            ZoneSettingsPatch {
                reset_name: true,
                safety_max_volume: Some(None),
                ..ZoneSettingsPatch::default()
            },
            super::Viewer::default(),
            None,
        )
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
                above_comfort: false,
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
                "device": "Mujin Music for Windows",
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
                repeat: RepeatMode::Off,
                shuffle: false,
            },
        )
        .expect("heartbeat");
    assert_eq!(
        pending.commands[0].command,
        ZoneCommand::SetVolume {
            volume: 60,
            member_id: None,
            above_comfort: false,
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
        service.run_sleep_timers().await;
        let zone = service
            .zone(USER, "client:browser-sleep1", None)
            .expect("zone");
        let timer = zone.sleep_timer.expect("timer kept while the track ends");
        assert!(timer.finishing_track);
        service.force_sleep_due("client:browser-sleep1");
        service.run_sleep_timers().await;
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

async fn single_sonos_room(
    room: &'static str,
    uuid: &'static str,
) -> (FakeSonos, Arc<SonosController>, ZoneService) {
    let fake = FakeSonos::start(&[FakePlayerSpec {
        uuid,
        room,
        model: "Sonos One",
        pair_of: None,
    }])
    .await;
    let sonos = Arc::new(
        SonosController::new(SonosConfig {
            ssdp_targets: vec![fake.ssdp_address],
            static_hosts: Vec::new(),
            search_timeout: Duration::from_millis(400),
        })
        .expect("controller"),
    );
    assert_eq!(sonos.discover().await.expect("discovery"), 1);
    let service = ZoneService::new(Some(sonos.clone()), None);
    (fake, sonos, service)
}

fn queue_titles(fake: &FakeSonos, uuid: &str) -> Vec<String> {
    fake.player(uuid)
        .queue
        .iter()
        .map(|(uri, _)| {
            uri.split("/zones/media/")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

#[actix_web::test]
async fn sonos_queue_is_managed_remotely_through_zone_commands() {
    const DEN: &str = "RINCON_DEN000001400";
    let (fake, _sonos, service) = single_sonos_room("Den", DEN).await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let cache = library(&directory);
    let zone_id = format!("sonos:{DEN}");
    let all: Vec<String> = ["song-1", "song-2", "song-3", "song-4"]
        .map(String::from)
        .to_vec();
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Load {
                song_ids: all.clone(),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert!(zone.capabilities.queue_edit && zone.capabilities.repeat);
    assert!(zone.capabilities.shuffle);
    let removes_before = fake.calls("RemoveAllTracksFromQueue").len();

    // Drag the last song up next: a native Sonos reorder, no reload.
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::MoveQueueItem {
                from: 3,
                to: 1,
                song_id: Some("song-4".into()),
            },
            Some(&cache),
        )
        .await
        .expect("move");
    assert_eq!(
        queue_titles(&fake, DEN),
        ["song-1", "song-4", "song-2", "song-3"]
    );
    assert_eq!(fake.calls("ReorderTracksInQueue").len(), 1);
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert_eq!(
        zone.queue.song_ids,
        ["song-1", "song-4", "song-2", "song-3"]
    );
    assert_eq!(fake.player(DEN).track, 1, "the playing track is untouched");
    assert_eq!(fake.player(DEN).state, "PLAYING");

    // A remote acting on a stale queue is refused instead of removing the
    // wrong song.
    assert_eq!(
        service
            .command(
                USER,
                &zone_id,
                ZoneCommand::RemoveQueueItem {
                    queue_position: 2,
                    song_id: Some("song-3".into()),
                },
                Some(&cache),
            )
            .await,
        Err(ZoneError::Invalid("queue_changed"))
    );
    // The playing track cannot be removed from under the speaker.
    assert_eq!(
        service
            .command(
                USER,
                &zone_id,
                ZoneCommand::RemoveQueueItem {
                    queue_position: 0,
                    song_id: None,
                },
                Some(&cache),
            )
            .await,
        Err(ZoneError::Invalid("only_upcoming_items_remove"))
    );
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::RemoveQueueItem {
                queue_position: 2,
                song_id: Some("song-2".into()),
            },
            Some(&cache),
        )
        .await
        .expect("remove");
    assert_eq!(queue_titles(&fake, DEN), ["song-1", "song-4", "song-3"]);
    assert!(
        fake.calls("RemoveTrackFromQueue")
            .iter()
            .any(|call| call.arguments["ObjectID"] == "Q:0/3")
    );
    assert!(
        !service.media_allowed(DEN, "song-2"),
        "removed songs stop streaming"
    );
    assert!(service.media_allowed(DEN, "song-3"));

    // Repeat maps onto the Sonos play mode and is reported back.
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::SetRepeat {
                mode: RepeatMode::All,
            },
            Some(&cache),
        )
        .await
        .expect("repeat");
    assert_eq!(fake.player(DEN).play_mode, "REPEAT_ALL");
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert_eq!(zone.repeat, RepeatMode::All);

    // Shuffle reorders only what is upcoming, on the speaker too, and
    // turning it off restores the order.
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::SetShuffle { enabled: true },
            Some(&cache),
        )
        .await
        .expect("shuffle");
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert!(zone.shuffle);
    assert_eq!(zone.queue.song_ids[0], "song-1");
    assert_eq!(queue_titles(&fake, DEN), zone.queue.song_ids);
    let mut shuffled = zone.queue.song_ids.clone();
    shuffled.sort();
    assert_eq!(shuffled, ["song-1", "song-3", "song-4"]);
    assert_eq!(fake.player(DEN).track, 1);
    assert_eq!(fake.player(DEN).play_mode, "REPEAT_ALL", "repeat survives");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::SetShuffle { enabled: false },
            Some(&cache),
        )
        .await
        .expect("unshuffle");
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert!(!zone.shuffle);
    assert_eq!(zone.queue.song_ids, ["song-1", "song-4", "song-3"]);
    assert_eq!(queue_titles(&fake, DEN), ["song-1", "song-4", "song-3"]);

    // Clear upcoming leaves only the playing track, still playing.
    service
        .command(USER, &zone_id, ZoneCommand::ClearUpcoming, Some(&cache))
        .await
        .expect("clear");
    assert_eq!(queue_titles(&fake, DEN), ["song-1"]);
    assert_eq!(fake.player(DEN).state, "PLAYING");
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert_eq!(zone.queue.song_ids, ["song-1"]);
    assert_eq!(
        fake.calls("RemoveAllTracksFromQueue").len(),
        removes_before,
        "no edit reloaded the whole queue"
    );

    // With repeat-all on, the speaker starts over after the last track.
    fake.advance(DEN);
    assert_eq!(fake.player(DEN).track, 1);

    // Play next and Add to queue from the library: the speaker's own queue
    // grows without the playing track stopping.
    let adds_before = fake.calls("AddURIToQueue").len();
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::AddToQueue {
                song_ids: vec!["song-3".into()],
                next: true,
            },
            Some(&cache),
        )
        .await
        .expect("play next");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::AddToQueue {
                song_ids: vec!["song-2".into()],
                next: false,
            },
            Some(&cache),
        )
        .await
        .expect("add to queue");
    assert_eq!(queue_titles(&fake, DEN), ["song-1", "song-3", "song-2"]);
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert_eq!(zone.queue.song_ids, ["song-1", "song-3", "song-2"]);
    assert_eq!(fake.player(DEN).state, "PLAYING");
    assert_eq!(fake.player(DEN).track, 1, "the playing track carries on");
    assert!(
        fake.calls("AddURIToQueue").len() > adds_before,
        "added natively"
    );
    assert_eq!(
        fake.calls("RemoveAllTracksFromQueue").len(),
        removes_before,
        "adding never reloads the whole queue"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn client_zone_queue_edits_show_at_once_and_reach_the_client() {
    let service = ZoneService::new(None, None);
    let queue: Vec<String> = ["song-1", "song-2", "song-3", "song-4"]
        .map(String::from)
        .to_vec();
    service
        .register_client(
            USER,
            "browser-edit01",
            web_client(ZoneStatus::Playing, Some(queue)),
        )
        .expect("register");
    let zone_id = "client:browser-edit01";
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::MoveQueueItem {
                from: 3,
                to: 2,
                song_id: Some("song-4".into()),
            },
            None,
        )
        .await
        .expect("move");
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::SetRepeat {
                mode: RepeatMode::One,
            },
            None,
        )
        .await
        .expect("repeat");
    let zone = service.zone(USER, zone_id, None).expect("zone");
    assert_eq!(
        zone.queue.song_ids,
        ["song-1", "song-2", "song-4", "song-3"]
    );
    assert_eq!(zone.repeat, RepeatMode::One);
    assert_eq!(
        service
            .command(
                USER,
                zone_id,
                ZoneCommand::RemoveQueueItem {
                    queue_position: 1,
                    song_id: None,
                },
                None,
            )
            .await,
        Err(ZoneError::Invalid("only_upcoming_items_remove")),
        "current track is position 1"
    );
    let mut heartbeat = web_client(ZoneStatus::Playing, None);
    heartbeat.repeat = RepeatMode::One;
    let pending = service
        .register_client(USER, "browser-edit01", heartbeat)
        .expect("heartbeat");
    assert!(pending.needs_queue, "the client confirms its real queue");
    assert_eq!(
        pending
            .commands
            .iter()
            .map(|command| command.command.clone())
            .collect::<Vec<_>>(),
        vec![
            ZoneCommand::MoveQueueItem {
                from: 3,
                to: 2,
                song_id: Some("song-4".into())
            },
            ZoneCommand::SetRepeat {
                mode: RepeatMode::One
            },
        ]
    );

    // Play next from a remote shows at once and reaches the client.
    service
        .command(
            USER,
            zone_id,
            ZoneCommand::AddToQueue {
                song_ids: vec!["song-9".into()],
                next: true,
            },
            None,
        )
        .await
        .expect("play next on the client");
    let zone = service.zone(USER, zone_id, None).expect("zone");
    assert_eq!(
        zone.queue.song_ids[2], "song-9",
        "right after the current track"
    );
    let pending = service
        .register_client(
            USER,
            "browser-edit01",
            web_client(ZoneStatus::Playing, None),
        )
        .expect("heartbeat");
    assert!(pending.commands.iter().any(|command| command.command
        == ZoneCommand::AddToQueue {
            song_ids: vec!["song-9".into()],
            next: true,
        }));

    // A client with nothing queued starts the songs instead.
    service
        .register_client(
            USER,
            "browser-idle01",
            web_client(ZoneStatus::Idle, Some(Vec::new())),
        )
        .expect("register idle");
    service
        .command(
            USER,
            "client:browser-idle01",
            ZoneCommand::AddToQueue {
                song_ids: vec!["song-2".into()],
                next: false,
            },
            None,
        )
        .await
        .expect("add to an empty zone");
    let pending = service
        .register_client(USER, "browser-idle01", web_client(ZoneStatus::Idle, None))
        .expect("heartbeat");
    assert!(
        matches!(
            pending.commands.last().map(|command| &command.command),
            Some(ZoneCommand::Load { song_ids, play: true, .. }) if song_ids == &["song-2"]
        ),
        "{:?}",
        pending.commands
    );
}

#[actix_web::test]
async fn sonos_sleep_timer_fades_pauses_and_restores_volume() {
    const STUDY: &str = "RINCON_STUDY00001400";
    let (fake, sonos, service) = single_sonos_room("Study", STUDY).await;
    service.set_sleep_fade(Duration::from_millis(160));
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let mut cache = library(&directory);
    // A 15-minute first track: a timer running out early in it fades.
    Arc::make_mut(&mut cache.artists)[0].albums[0].songs[0].duration = 900.0;
    let zone_id = format!("sonos:{STUDY}");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Load {
                song_ids: vec!["song-1".into(), "song-2".into()],
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::SetVolume {
                volume: 40,
                member_id: None,
                above_comfort: false,
            },
            None,
        )
        .await
        .expect("volume");

    // Sonos rooms belong to no single account; the timer remembers who set
    // it instead of being dropped when it runs out.
    service
        .set_sleep_timer(USER, &zone_id, Some(30))
        .expect("timer");
    service.force_sleep_due(&zone_id);
    service.run_sleep_timers().await;
    let player = fake.player(STUDY);
    assert_eq!(player.state, "PAUSED_PLAYBACK", "the timer paused the room");
    assert_eq!(player.volume, 40, "volume is back for the next play");
    let levels: Vec<u8> = fake
        .calls("SetVolume")
        .iter()
        .filter_map(|call| call.arguments["DesiredVolume"].parse().ok())
        .collect();
    assert!(levels.contains(&0), "faded out: {levels:?}");
    assert!(levels.windows(2).any(|pair| pair[1] < pair[0]));
    let zone = service.zone(USER, &zone_id, Some(&cache)).expect("zone");
    assert!(zone.sleep_timer.is_none());

    // Near the end of a track the timer lets it finish, then pauses when
    // the next track starts, without fading.
    service
        .command(USER, &zone_id, ZoneCommand::Play, None)
        .await
        .expect("play");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Seek {
                position_ms: 880_000,
            },
            None,
        )
        .await
        .expect("seek");
    let fades = fake.calls("SetVolume").len();
    service
        .set_sleep_timer(USER, &zone_id, Some(1))
        .expect("timer");
    service.force_sleep_due(&zone_id);
    service.run_sleep_timers().await;
    let timer = service
        .zone(USER, &zone_id, Some(&cache))
        .and_then(|zone| zone.sleep_timer)
        .expect("finishing timer");
    assert!(timer.finishing_track);
    assert_eq!(fake.player(STUDY).state, "PLAYING");
    fake.advance(STUDY);
    let _ = sonos.poll_group(STUDY).await;
    service.run_sleep_timers().await;
    assert_eq!(fake.player(STUDY).state, "PAUSED_PLAYBACK");
    assert_eq!(
        fake.calls("SetVolume").len(),
        fades,
        "no fade after a finished track"
    );
    assert!(
        service
            .zone(USER, &zone_id, Some(&cache))
            .and_then(|zone| zone.sleep_timer)
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&directory);
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
    // Lossy decoders put out floating point: 24 bits keep all of it.
    assert_eq!(
        plan(&song("a.ogg", "opus", 48_000, None)).convert_to,
        Some((48_000, 24))
    );
    assert_eq!(
        plan(&song("a.ogg", "vorbis", 44_100, None)).convert_to,
        Some((44_100, 24))
    );
    // Lossless files Sonos does not take natively keep their own depth.
    assert_eq!(
        plan(&song("a.aiff", "aiff", 44_100, Some(16))).convert_to,
        Some((44_100, 16))
    );
}

fn set_audio(
    cache: &mut LibraryCache,
    song_index: usize,
    extension: &str,
    codec: &str,
    sample_rate: u32,
    bit_depth: Option<u8>,
) {
    let song = &mut Arc::make_mut(&mut cache.artists)[0].albums[0].songs[song_index];
    let path = std::path::Path::new(&song.path).with_extension(extension);
    std::fs::write(&path, vec![0x66_u8; 4_096]).expect("fixture audio");
    song.path = path.to_string_lossy().to_string();
    song.audio = Some(AudioProperties {
        codec: Some(codec.to_string()),
        container: Some(extension.to_string()),
        sample_rate: Some(sample_rate),
        bit_depth,
        channels: Some(2),
        bitrate_kbps: None,
        lossless: Some(bit_depth.is_some()),
    });
}

#[actix_web::test]
async fn sonos_signal_path_follows_the_playing_track_from_the_served_plan() {
    const STUDIO: &str = "RINCON_STUDIO01400";
    let (fake, sonos, service) = single_sonos_room("Studio", STUDIO).await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let mut cache = library(&directory);
    set_audio(&mut cache, 0, "flac", "flac", 96_000, Some(24));
    set_audio(&mut cache, 1, "ogg", "vorbis", 44_100, None);
    set_audio(&mut cache, 2, "flac", "flac", 44_100, Some(16));
    let zone_id = format!("sonos:{STUDIO}");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Load {
                song_ids: ["song-1", "song-2", "song-3"].map(String::from).to_vec(),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    let stream = |service: &ZoneService| {
        service
            .zone(USER, &zone_id, Some(&cache))
            .expect("zone")
            .output
            .expect("output")
            .stream
            .expect("signal path while Mujin Music plays")
    };

    // Hi-res FLAC: resampled into Sonos's 48 kHz ceiling, depth kept.
    let path = stream(&service);
    assert_eq!(path.source, "FLAC 96 kHz/24-bit");
    assert_eq!(
        path.steps,
        [
            "Sample rate conversion 96 kHz to 48 kHz (linear phase)",
            "Re-encoded as FLAC (lossless)"
        ]
    );
    assert_eq!(path.output, "FLAC 48 kHz/24-bit to Studio (Sonos One)");
    assert!(!path.bit_perfect);
    assert_eq!(path.quality, StreamQuality::High);
    // The speaker was promised exactly that FLAC.
    let (_, metadata) = fake.player(STUDIO).queue[0].clone();
    assert!(metadata.contains("audio/flac"), "{metadata}");

    // The path refreshes with the track: Ogg Vorbis is decoded to 24-bit.
    service
        .command(USER, &zone_id, ZoneCommand::Next, Some(&cache))
        .await
        .expect("next");
    sonos.poll_all().await;
    let path = stream(&service);
    assert_eq!(path.source, "Ogg Vorbis 44.1 kHz");
    assert_eq!(
        path.steps,
        [
            "Decoded Ogg Vorbis to 24-bit PCM",
            "Sent as lossless FLAC 44.1 kHz/24-bit"
        ]
    );
    assert_eq!(path.output, "FLAC 44.1 kHz/24-bit to Studio (Sonos One)");
    assert_eq!(path.quality, StreamQuality::Low);
    assert!(
        service
            .zone(USER, &zone_id, Some(&cache))
            .expect("zone")
            .output
            .expect("output")
            .note
            .contains("lossy files (Ogg Vorbis, Opus) are decoded to 24-bit"),
        "the note matches what the speaker receives"
    );

    // CD-quality FLAC within the limit reaches the speaker untouched.
    service
        .command(USER, &zone_id, ZoneCommand::Next, Some(&cache))
        .await
        .expect("next");
    sonos.poll_all().await;
    let path = stream(&service);
    assert!(path.bit_perfect);
    assert!(path.steps.is_empty());
    assert_eq!(path.quality, StreamQuality::Lossless);
    assert_eq!(
        path.output,
        "Original FLAC file, untouched, to Studio (Sonos One)"
    );

    // Leveling: converted so the gain can be applied, shown while the
    // loudness is measured.
    service
        .update_settings(
            USER,
            &zone_id,
            ZoneSettingsPatch {
                leveling: Some(LevelingMode::Track),
                ..ZoneSettingsPatch::default()
            },
            super::Viewer::default(),
            Some(&cache),
        )
        .await
        .expect("leveling");
    let path = stream(&service);
    assert!(!path.bit_perfect);
    assert_eq!(path.quality, StreamQuality::Enhanced);
    assert_eq!(
        path.steps[0],
        "Volume leveling (measuring loudness; the gain shows here once measured)"
    );
    assert!(path.steps.contains(
        &"Carried at 24-bit (from 16-bit) so the processing keeps full resolution".to_string()
    ));
    let _ = std::fs::remove_dir_all(&directory);
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

fn set_duration(cache: &mut LibraryCache, song_index: usize, seconds: f64) {
    Arc::make_mut(&mut cache.artists)[0].albums[0].songs[song_index].duration = seconds;
}

async fn sonos_pair_of_rooms() -> (FakeSonos, Arc<SonosController>, ZoneService) {
    let fake = FakeSonos::start(&[
        FakePlayerSpec {
            uuid: LIVING,
            room: "Living Room",
            model: "Sonos Five",
            pair_of: None,
        },
        FakePlayerSpec {
            uuid: KITCHEN,
            room: "Kitchen",
            model: "Sonos One",
            pair_of: None,
        },
    ])
    .await;
    let sonos = Arc::new(
        SonosController::new(SonosConfig {
            ssdp_targets: vec![fake.ssdp_address],
            static_hosts: Vec::new(),
            search_timeout: Duration::from_millis(400),
        })
        .expect("controller"),
    );
    assert_eq!(sonos.discover().await.expect("discovery"), 2);
    let service = ZoneService::new(Some(sonos.clone()), None);
    (fake, sonos, service)
}

/// Regression: a remote Sonos zone, and the group it joins, report the
/// queued track's own length (B/05a and B/08 showed 4:16 for a 3:20 track).
#[actix_web::test]
async fn transferred_and_grouped_sonos_zones_report_the_tracks_own_duration() {
    let (fake, sonos, service) = sonos_pair_of_rooms().await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let mut cache = library(&directory);
    set_duration(&mut cache, 0, 256.0);
    set_duration(&mut cache, 1, 200.0);
    let living_id = format!("sonos:{LIVING}");
    let kitchen_id = format!("sonos:{KITCHEN}");
    let queue: Vec<String> = ["song-1", "song-2", "song-3"].map(String::from).to_vec();
    service
        .register_client(
            USER,
            "browser-duration",
            web_client(ZoneStatus::Playing, Some(queue)),
        )
        .expect("register");
    service
        .transfer(USER, "client:browser-duration", &living_id, Some(&cache))
        .await
        .expect("transfer");
    sonos.poll_all().await;
    let metadata_ms = (cache.song("song-2").expect("song").duration * 1_000.0) as i64;
    assert_eq!(metadata_ms, 200_000);
    let duration_of = |zone_id: &str| {
        let zone = service.zone(USER, zone_id, Some(&cache)).expect("zone");
        assert_eq!(
            zone.now_playing.as_ref().map(|now| now.title.as_str()),
            Some("Second <Song>")
        );
        assert!(zone.position_ms <= zone.duration_ms);
        (
            zone.duration_ms,
            zone.now_playing.map(|now| now.duration_ms),
        )
    };
    assert_eq!(duration_of(&living_id), (200_000, Some(200_000)));

    // A speaker still reporting the previous track's length (4:16) does
    // not leak into the readout.
    fake.report_duration(LIVING, Some(256_000));
    sonos.poll_all().await;
    assert_eq!(duration_of(&living_id), (200_000, Some(200_000)));

    // Native Sonos group: the group zone shows the same track length.
    let group_id = service
        .group(USER, &[living_id.clone(), kitchen_id.clone()])
        .await
        .expect("group");
    sonos.poll_all().await;
    let group = service.zone(USER, &group_id, Some(&cache)).expect("group");
    assert_eq!(group.group.as_ref().map(|info| info.members.len()), Some(2));
    assert_eq!(duration_of(&group_id), (200_000, Some(200_000)));
    service
        .ungroup(USER, &group_id, None)
        .await
        .expect("ungroup");

    // Linked group led by the Sonos room: same again.
    let linked = service
        .group(
            USER,
            &[living_id.clone(), "client:browser-duration".to_string()],
        )
        .await
        .expect("linked");
    assert_eq!(duration_of(&linked), (200_000, Some(200_000)));
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn sonos_volume_limits_crossfade_and_leveling_reach_the_speaker() {
    const DEN: &str = "RINCON_DEN000002400";
    let (fake, _sonos, service) = single_sonos_room("Den", DEN).await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let cache = library(&directory);
    let zone_id = format!("sonos:{DEN}");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Load {
                song_ids: ["song-1", "song-2", "song-3"].map(String::from).to_vec(),
                current_position: 0,
                position_ms: 0,
                play: true,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    let settings = |patch: ZoneSettingsPatch| {
        let service = &service;
        let zone_id = zone_id.clone();
        let cache = &cache;
        async move {
            service
                .update_settings(USER, &zone_id, patch, super::Viewer::default(), Some(cache))
                .await
        }
    };
    settings(ZoneSettingsPatch {
        comfort_volume: Some(Some(30)),
        safety_min_volume: Some(Some(5)),
        safety_max_volume: Some(Some(60)),
        ..ZoneSettingsPatch::default()
    })
    .await
    .expect("limits");
    let volume = |command: ZoneCommand| {
        let service = &service;
        let zone_id = zone_id.clone();
        async move { service.command(USER, &zone_id, command, None).await }
    };
    // The slider (absolute) stops at the comfort limit...
    volume(ZoneCommand::SetVolume {
        volume: 50,
        member_id: None,
        above_comfort: false,
    })
    .await
    .expect("set");
    assert_eq!(fake.player(DEN).volume, 30);
    // ...the + button goes past it, up to the safety maximum.
    volume(ZoneCommand::AdjustVolume {
        delta: 20,
        member_id: None,
    })
    .await
    .expect("step");
    assert_eq!(fake.player(DEN).volume, 50);
    volume(ZoneCommand::AdjustVolume {
        delta: 20,
        member_id: None,
    })
    .await
    .expect("step");
    assert_eq!(fake.player(DEN).volume, 60);
    volume(ZoneCommand::SetVolume {
        volume: 0,
        member_id: None,
        above_comfort: false,
    })
    .await
    .expect("low");
    assert_eq!(fake.player(DEN).volume, 5, "safety minimum holds");
    volume(ZoneCommand::AdjustVolume {
        delta: 40,
        member_id: None,
    })
    .await
    .expect("step");
    // Tightening the safety maximum pulls the speaker down at once.
    settings(ZoneSettingsPatch {
        comfort_volume: Some(None),
        safety_max_volume: Some(Some(35)),
        ..ZoneSettingsPatch::default()
    })
    .await
    .expect("tighter");
    assert_eq!(fake.player(DEN).volume, 35);
    let zone = service.zone(USER, &zone_id, None).expect("zone");
    let reported = zone.volume.expect("volume");
    assert_eq!(
        (reported.min, reported.max, reported.comfort),
        (5, 35, None)
    );
    assert_eq!((reported.device_min, reported.device_max), (0, 100));

    // Crossfade maps onto Sonos crossfade.
    settings(ZoneSettingsPatch {
        crossfade_seconds: Some(6),
        ..ZoneSettingsPatch::default()
    })
    .await
    .expect("crossfade");
    assert!(fake.player(DEN).crossfade);
    settings(ZoneSettingsPatch {
        crossfade_seconds: Some(0),
        ..ZoneSettingsPatch::default()
    })
    .await
    .expect("no crossfade");
    assert!(!fake.player(DEN).crossfade);

    // Leveling re-sends the upcoming tracks with signed leveling; the
    // playing track is left alone.
    let current_before = fake.player(DEN).queue[0].0.clone();
    settings(ZoneSettingsPatch {
        leveling: Some(LevelingMode::Auto),
        leveling_target_lufs: Some(-16),
        ..ZoneSettingsPatch::default()
    })
    .await
    .expect("leveling");
    let player = fake.player(DEN);
    assert_eq!(player.queue.len(), 3);
    assert_eq!(player.queue[0].0, current_before);
    assert_eq!(player.track, 1);
    for (uri, metadata) in &player.queue[1..] {
        assert!(
            uri.ends_with("&level=album&target=-16&adjust=0"),
            "auto levels an album played in order: {uri}"
        );
        assert!(metadata.contains("http-get:*:audio/flac:*"));
    }
    let zone = service.zone(USER, &zone_id, None).expect("zone");
    assert_eq!(zone.settings.leveling, LevelingMode::Auto);
    assert_eq!(zone.settings.leveling_target_lufs, -16);
    assert!(zone.settings.supports.crossfade && zone.settings.supports.leveling);
    assert!(!zone.settings.supports.private);
    assert_eq!(
        settings(ZoneSettingsPatch {
            private: Some(true),
            ..ZoneSettingsPatch::default()
        })
        .await,
        Err(ZoneError::Unsupported("only_parson_apps_can_be_private"))
    );
    let _ = std::fs::remove_dir_all(&directory);
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Leveling end to end: the speaker fetches a leveled URL, the server
/// measures the track (EBU R128) and serves it at the target loudness.
#[actix_web::test]
async fn leveled_sonos_stream_reaches_the_target_loudness() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg is not installed; leveling needs it to measure loudness");
        return;
    }
    const DEN: &str = "RINCON_DEN000003400";
    let (fake, _sonos, service) = single_sonos_room("Den", DEN).await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let mut cache = library(&directory);
    // Two real tracks, one loud and one quiet.
    for (index, volume) in [(0_usize, "-2dB"), (1, "-20dB")] {
        let path = directory.join(format!("tone-{index}.flac"));
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:sample_rate=44100:duration=6")
            .args(["-af", &format!("volume={volume}"), "-c:a", "flac"])
            .arg(&path)
            .status()
            .expect("ffmpeg");
        assert!(status.success());
        let song = &mut Arc::make_mut(&mut cache.artists)[0].albums[0].songs[index];
        song.path = path.to_string_lossy().to_string();
        song.duration = 6.0;
        song.audio = Some(AudioProperties {
            codec: Some("flac".to_string()),
            container: Some("flac".to_string()),
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            channels: Some(1),
            bitrate_kbps: None,
            lossless: Some(true),
        });
    }
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle.set_ready(cache).await;
    let cache = lifecycle.cache().await.expect("cache");
    let service = web::Data::new(service);
    let server_service = service.clone();
    let server_lifecycle = web::Data::from(lifecycle.clone());
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
    let zone_id = format!("sonos:{DEN}");
    service
        .update_settings(
            USER,
            &zone_id,
            ZoneSettingsPatch {
                leveling: Some(LevelingMode::Track),
                ..ZoneSettingsPatch::default()
            },
            super::Viewer::default(),
            Some(&cache),
        )
        .await
        .expect("leveling");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Load {
                song_ids: vec!["song-1".into(), "song-2".into()],
                current_position: 0,
                position_ms: 0,
                play: false,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("client");
    let mut levels = Vec::new();
    for (uri, _) in fake.player(DEN).queue {
        assert!(uri.contains("&level=track&target=-18&adjust=0"), "{uri}");
        // The leveling is signed: changing it in the URL is refused.
        let tampered = uri.replace("target=-18", "target=-5");
        assert_eq!(
            http.get(&tampered).send().await.expect("tampered").status(),
            401
        );
        let response = http.get(&uri).send().await.expect("leveled stream");
        assert_eq!(response.status(), 200);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("audio/flac")
        );
        let bytes = response.bytes().await.expect("audio");
        let output = directory.join(format!("leveled-{}.flac", levels.len()));
        std::fs::write(&output, &bytes).expect("write leveled audio");
        let measured = super::leveling::measure_file(&output)
            .await
            .expect("measure leveled audio");
        levels.push(measured);
    }
    // Both tracks were pulled toward -18 LUFS, but the quiet one is only
    // raised as far as its peak allows (true peak at most -1 dBTP).
    let loud = levels[0];
    let quiet = levels[1];
    assert!(
        (loud.integrated_lufs + 18.0).abs() < 1.0,
        "loud track leveled to {loud:?}"
    );
    assert!(loud.true_peak_db <= -0.5, "{loud:?}");
    assert!(quiet.true_peak_db <= -0.5, "{quiet:?}");
    let original_quiet = super::leveling::measure_file(&directory.join("tone-1.flac"))
        .await
        .expect("measure original");
    assert!(
        quiet.integrated_lufs > original_quiet.integrated_lufs + 5.0,
        "quiet track raised: {original_quiet:?} -> {quiet:?}"
    );
    // Now measured, the zone's signal path shows the gain it applies.
    let gain = service
        .loudness()
        .gain_for(&cache, "song-1", super::leveling::LevelBy::Track, -18, 0)
        .await
        .expect("gain");
    let path = service
        .zone(USER, &zone_id, Some(&cache))
        .expect("zone")
        .output
        .and_then(|output| output.stream)
        .expect("signal path");
    assert_eq!(path.steps[0], format!("Volume leveling {gain:+.1} dB"));
    assert!(gain.abs() >= 1.0, "a real gain is applied: {gain}");
    assert_eq!(path.quality, StreamQuality::Enhanced);
    server_handle.stop(false).await;
    let _ = std::fs::remove_dir_all(&directory);
}

/// What the speaker actually receives for a lossy file: FLAC at 24 bits,
/// as its signal path and format note say.
#[actix_web::test]
async fn sonos_gets_ogg_and_opus_as_24_bit_flac() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg is not installed; conversions need it");
        return;
    }
    const DEN: &str = "RINCON_DEN000004400";
    let (fake, _sonos, service) = single_sonos_room("Den", DEN).await;
    let directory = std::env::temp_dir().join(format!("parson-zones-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    let mut cache = library(&directory);
    for (index, extension, encoder, codec, rate) in [
        (0_usize, "ogg", "libvorbis", "vorbis", 44_100_u32),
        (1, "opus", "libopus", "opus", 48_000),
    ] {
        let path = directory.join(format!("tone-{index}.{extension}"));
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:sample_rate={rate}:duration=2"))
            .args(["-c:a", encoder])
            .arg(&path)
            .status()
            .expect("ffmpeg");
        assert!(status.success());
        let song = &mut Arc::make_mut(&mut cache.artists)[0].albums[0].songs[index];
        song.path = path.to_string_lossy().to_string();
        song.duration = 2.0;
        song.audio = Some(AudioProperties {
            codec: Some(codec.to_string()),
            container: Some(extension.to_string()),
            sample_rate: Some(rate),
            bit_depth: None,
            channels: Some(1),
            bitrate_kbps: Some(128),
            lossless: Some(false),
        });
    }
    let lifecycle = Arc::new(LibraryLifecycle::new());
    lifecycle.set_ready(cache).await;
    let cache = lifecycle.cache().await.expect("cache");
    let service = web::Data::new(service);
    let server_service = service.clone();
    let server_lifecycle = web::Data::from(lifecycle.clone());
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
    let zone_id = format!("sonos:{DEN}");
    service
        .command(
            USER,
            &zone_id,
            ZoneCommand::Load {
                song_ids: vec!["song-1".into(), "song-2".into()],
                current_position: 0,
                position_ms: 0,
                play: false,
            },
            Some(&cache),
        )
        .await
        .expect("load");
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("client");
    let mut received = Vec::new();
    for (index, (uri, metadata)) in fake.player(DEN).queue.into_iter().enumerate() {
        assert!(metadata.contains("audio/flac"), "{metadata}");
        let response = http.get(&uri).send().await.expect("stream");
        assert_eq!(response.status(), 200);
        let bytes = response.bytes().await.expect("audio");
        let output = directory.join(format!("received-{index}.flac"));
        std::fs::write(&output, &bytes).expect("write received audio");
        let probe = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "a:0", "-show_entries"])
            .args([
                "stream=codec_name,sample_rate,bits_per_raw_sample",
                "-of",
                "csv=p=0",
            ])
            .arg(&output)
            .output()
            .expect("ffprobe");
        received.push(String::from_utf8_lossy(&probe.stdout).trim().to_string());
    }
    assert_eq!(received, ["flac,44100,24", "flac,48000,24"]);
    server_handle.stop(false).await;
    let _ = std::fs::remove_dir_all(&directory);
}

#[actix_web::test]
async fn zone_settings_change_only_what_is_sent_and_private_zones_stay_private() {
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
    for (device, kind, name) in [
        ("desktop-private1", "desktop", "Studio PC"),
        ("phone-remote0001", "mobile", "Pixel"),
    ] {
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::put()
                .uri(&format!("/zones/clients/{device}"))
                .set_json(serde_json::json!({
                    "kind": kind, "name": name, "status": "playing",
                    "song_ids": ["song-1"], "volume": 50
                }))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
    }
    let studio = "/zones/client:desktop-private1/settings";
    let patch = |body: serde_json::Value, device: Option<&'static str>| {
        let mut request = actix_test::TestRequest::patch().uri(studio).set_json(body);
        if let Some(device) = device {
            request = request.insert_header((crate::api::zones::ZONE_DEVICE_HEADER, device));
        }
        request.to_request()
    };
    let name_of = |body: &serde_json::Value| body["zone"]["name"].as_str().map(str::to_string);

    let renamed: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        patch(serde_json::json!({"name": "Studio"}), None),
    )
    .await;
    assert_eq!(name_of(&renamed).as_deref(), Some("Studio"));
    assert_eq!(renamed["zone"]["settings"]["custom_name"], true);
    // Saving other settings leaves the name alone, with or without a null.
    for body in [
        serde_json::json!({"crossfade_seconds": 4}),
        serde_json::json!({"name": null, "leveling": "album", "leveling_target_lufs": -14}),
        serde_json::json!({"icon": "headphones", "comfort_volume": 40, "safety_max_volume": 80}),
    ] {
        let saved: serde_json::Value =
            actix_test::call_and_read_body_json(&app, patch(body.clone(), None)).await;
        assert_eq!(name_of(&saved).as_deref(), Some("Studio"), "{body}");
    }
    let listed: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get().uri("/zones").to_request(),
    )
    .await;
    let studio_zone = listed["zones"]
        .as_array()
        .and_then(|zones| {
            zones
                .iter()
                .find(|zone| zone["id"] == "client:desktop-private1")
        })
        .cloned()
        .expect("studio zone");
    assert_eq!(studio_zone["icon"], "headphones");
    assert_eq!(studio_zone["settings"]["crossfade_seconds"], 4);
    assert_eq!(studio_zone["settings"]["leveling"], "album");
    assert_eq!(studio_zone["settings"]["leveling_target_lufs"], -14);
    assert_eq!(studio_zone["volume"]["comfort"], 40);
    assert_eq!(studio_zone["volume"]["max"], 80);
    // Only an explicit reset goes back to the device's name.
    let reset: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        patch(serde_json::json!({"reset_name": true}), None),
    )
    .await;
    assert_eq!(name_of(&reset).as_deref(), Some("Studio PC"));
    assert_eq!(reset["zone"]["settings"]["custom_name"], false);

    for (body, status) in [
        (serde_json::json!({"crossfade_seconds": 13}), 400),
        (serde_json::json!({"icon": "rocket"}), 400),
        (serde_json::json!({"name": "  "}), 400),
        (
            serde_json::json!({"safety_min_volume": 50, "safety_max_volume": 40}),
            400,
        ),
        (serde_json::json!({"comfort_volume": 90}), 400),
        (serde_json::json!({"leveling_target_lufs": -40}), 400),
        (serde_json::json!({"volume_cap": 10}), 400),
        // Private is set on the device itself.
        (serde_json::json!({"private": true}), 409),
    ] {
        let response = actix_test::call_service(&app, patch(body.clone(), None)).await;
        assert_eq!(response.status(), status, "{body}");
    }
    // The phone app cannot crossfade or level.
    let response = actix_test::call_service(
        &app,
        actix_test::TestRequest::patch()
            .uri("/zones/client:phone-remote0001/settings")
            .set_json(serde_json::json!({"crossfade_seconds": 3}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 409);

    // The studio PC makes itself private.
    let private: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        patch(
            serde_json::json!({"private": true}),
            Some("desktop-private1"),
        ),
    )
    .await;
    assert_eq!(private["zone"]["settings"]["private"], true);
    assert_eq!(private["zone"]["capabilities"]["group"], false);
    let ids = |listed: &serde_json::Value| -> Vec<String> {
        listed["zones"]
            .as_array()
            .map(|zones| {
                zones
                    .iter()
                    .filter_map(|zone| zone["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let from_phone: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/zones")
            .insert_header((crate::api::zones::ZONE_DEVICE_HEADER, "phone-remote0001"))
            .to_request(),
    )
    .await;
    assert_eq!(ids(&from_phone), ["client:phone-remote0001"]);
    let from_itself: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/zones")
            .insert_header((crate::api::zones::ZONE_DEVICE_HEADER, "desktop-private1"))
            .to_request(),
    )
    .await;
    assert!(ids(&from_itself).contains(&"client:desktop-private1".to_string()));
    // Other remotes can neither control nor group it.
    for request in [
        actix_test::TestRequest::post()
            .uri("/zones/client:desktop-private1/commands")
            .insert_header((crate::api::zones::ZONE_DEVICE_HEADER, "phone-remote0001"))
            .set_json(serde_json::json!({"command": "pause"}))
            .to_request(),
        actix_test::TestRequest::post()
            .uri("/zones/transfer")
            .set_json(serde_json::json!({
                "from": "client:desktop-private1", "to": "client:phone-remote0001"
            }))
            .to_request(),
        actix_test::TestRequest::post()
            .uri("/zones/groups")
            .set_json(serde_json::json!({
                "zone_ids": ["client:desktop-private1", "client:phone-remote0001"]
            }))
            .to_request(),
    ] {
        assert_eq!(actix_test::call_service(&app, request).await.status(), 404);
    }
    // Even its own device cannot group it.
    let response = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri("/zones/groups")
            .insert_header((crate::api::zones::ZONE_DEVICE_HEADER, "desktop-private1"))
            .set_json(serde_json::json!({
                "zone_ids": ["client:desktop-private1", "client:phone-remote0001"]
            }))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 409);
    // Its own device still controls it.
    let response = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri("/zones/client:desktop-private1/commands")
            .insert_header((crate::api::zones::ZONE_DEVICE_HEADER, "desktop-private1"))
            .set_json(serde_json::json!({"command": "pause"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 202);
}

#[test]
fn zone_settings_persist_across_restarts() {
    use diesel_migrations::MigrationHarness;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let pool = test_pool();
    pool.get()
        .expect("connection")
        .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
        .expect("migrations");
    runtime.block_on(async {
        let service = ZoneService::new(None, Some(pool.clone()));
        service
            .register_client(USER, "browser-persist", web_client(ZoneStatus::Idle, None))
            .expect("register");
        service
            .update_settings(
                USER,
                "client:browser-persist",
                ZoneSettingsPatch {
                    name: Some("Den laptop".into()),
                    icon: Some(Some("sofa".into())),
                    crossfade_seconds: Some(5),
                    leveling: Some(LevelingMode::Track),
                    leveling_target_lufs: Some(-20),
                    leveling_adjustment_db: Some(2),
                    comfort_volume: Some(Some(45)),
                    safety_min_volume: Some(Some(10)),
                    safety_max_volume: Some(Some(70)),
                    ..ZoneSettingsPatch::default()
                },
                super::Viewer::default(),
                None,
            )
            .await
            .expect("settings");
        let restarted = ZoneService::new(None, Some(pool.clone()));
        restarted.load_settings();
        restarted
            .register_client(USER, "browser-persist", web_client(ZoneStatus::Idle, None))
            .expect("register again");
        let zone = restarted
            .zone(USER, "client:browser-persist", None)
            .expect("zone");
        assert_eq!(zone.name, "Den laptop");
        assert_eq!(zone.icon, "sofa");
        assert_eq!(zone.settings.crossfade_seconds, 5);
        assert_eq!(zone.settings.leveling, LevelingMode::Track);
        assert_eq!(
            (
                zone.settings.leveling_target_lufs,
                zone.settings.leveling_adjustment_db
            ),
            (-20, 2)
        );
        let volume = zone.volume.expect("volume");
        assert_eq!((volume.min, volume.max, volume.comfort), (10, 70, Some(45)));
    });
}

#[actix_web::test]
async fn home_assistant_volume_control_sends_requests_as_events_instead_of_changing_volume() {
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
    let zone_id = "client:desktop-amp00001";
    let heartbeat = |acknowledged: u64| {
        actix_test::TestRequest::put()
            .uri("/zones/clients/desktop-amp00001")
            .set_json(serde_json::json!({
                "kind": "desktop", "name": "Living room PC", "status": "playing",
                "song_ids": ["song-1"], "volume": 70, "acknowledged_seq": acknowledged
            }))
            .to_request()
    };
    let registered: serde_json::Value =
        actix_test::call_and_read_body_json(&app, heartbeat(0)).await;
    assert_eq!(registered["commands"], serde_json::json!([]));

    let settings: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::patch()
            .uri(&format!("/zones/{zone_id}/settings"))
            .set_json(serde_json::json!({"volume_control": "home_assistant"}))
            .to_request(),
    )
    .await;
    let zone = &settings["zone"];
    assert_eq!(zone["settings"]["volume_control"], "home_assistant");
    assert_eq!(zone["settings"]["supports"]["volume_control"], true);
    assert_eq!(zone["volume"]["control"], "home_assistant");
    assert_eq!(
        zone["volume"]["level"], 70,
        "the estimate starts at the zone's own level"
    );

    let mut events = service.subscribe();
    let send = async |command: serde_json::Value| {
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri(&format!("/zones/{zone_id}/commands"))
                .set_json(command)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 202);
    };
    send(serde_json::json!({"command": "adjust_volume", "delta": 4})).await;
    send(serde_json::json!({"command": "adjust_volume", "delta": -2})).await;
    send(serde_json::json!({"command": "set_volume", "volume": 30})).await;
    send(serde_json::json!({"command": "set_mute", "muted": true})).await;
    let mut requests = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let ZoneEvent::VolumeRequest { user_id, request } = event {
            assert_eq!(
                user_id,
                Some(USER),
                "an app zone's requests stay with its owner"
            );
            requests.push((request.action, request.step, request.level, request.muted));
        }
    }
    assert_eq!(
        requests,
        vec![
            ("volume_up", 4, 74, false),
            ("volume_down", 2, 72, false),
            ("volume_set", 0, 30, false),
            ("mute_on", 0, 30, true),
        ]
    );
    // The app itself never got a volume command: its output stays as set.
    let after: serde_json::Value = actix_test::call_and_read_body_json(&app, heartbeat(0)).await;
    assert_eq!(after["commands"], serde_json::json!([]));
    let listed: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get().uri("/zones").to_request(),
    )
    .await;
    let zone = listed["zones"]
        .as_array()
        .and_then(|zones| zones.iter().find(|zone| zone["id"] == zone_id))
        .expect("zone listed");
    assert_eq!(zone["volume"]["level"], 30);
    assert_eq!(zone["volume"]["muted"], true);

    // Back to Parson: volume commands reach the app again.
    actix_test::call_service(
        &app,
        actix_test::TestRequest::patch()
            .uri(&format!("/zones/{zone_id}/settings"))
            .set_json(serde_json::json!({"volume_control": "parson"}))
            .to_request(),
    )
    .await;
    send(serde_json::json!({"command": "set_volume", "volume": 40})).await;
    let after: serde_json::Value = actix_test::call_and_read_body_json(&app, heartbeat(0)).await;
    assert_eq!(after["commands"][0]["command"]["command"], "set_volume");
    assert_eq!(after["commands"][0]["command"]["volume"], 40);
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, ZoneEvent::VolumeRequest { .. }),
            "no event once Mujin Music controls the volume again"
        );
    }

    // Standby is for zones that hold a device; an app zone refuses it.
    let response = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{zone_id}/commands"))
            .set_json(serde_json::json!({"command": "standby"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 409);
    let refused: serde_json::Value = actix_test::read_body_json(response).await;
    assert_eq!(refused["error"], "zone_has_no_standby");
    assert_eq!(zone["capabilities"]["standby"], false);
}
