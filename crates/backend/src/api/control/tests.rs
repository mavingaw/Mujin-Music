//! Controller API tests: browse tree, search, play by id and by search,
//! radio and thumbs, driven through the HTTP routes against a client zone.

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::dev::Service as _;
use actix_web::test as actix_test;
use actix_web::{App, HttpMessage, web};
use diesel::RunQueryDsl;
use diesel_migrations::MigrationHarness;

use super::*;
use crate::api::auth::Claims;
use crate::domain::{Album, Artist, AudioProperties, Song};
use crate::library::search::SearchIndex;
use crate::zones::{ClientRegistration, RepeatMode, ZoneKind, ZoneStatus};

const USER: i32 = 3;
const DEVICE: &str = "desktop-control1";
const ZONE: &str = "client:desktop-control1";

fn song(id: &str, name: &str, artist: &str, track: u16, hires: bool) -> Song {
    Song {
        id: id.to_string(),
        name: name.to_string(),
        artist: artist.to_string(),
        track_number: track,
        path: format!("/music/{id}.flac"),
        duration: 200.0,
        audio: Some(if hires {
            AudioProperties {
                codec: Some("flac".into()),
                container: Some("flac".into()),
                sample_rate: Some(96_000),
                bit_depth: Some(24),
                channels: Some(2),
                bitrate_kbps: Some(3_000),
                lossless: Some(true),
            }
        } else {
            AudioProperties {
                codec: Some("mp3".into()),
                container: Some("mp3".into()),
                sample_rate: Some(44_100),
                bit_depth: None,
                channels: Some(2),
                bitrate_kbps: Some(320),
                lossless: Some(false),
            }
        }),
        ..Song::default()
    }
}

/// Two artists, three albums, two genres. Album tracks are stored out of
/// order so play order must come from track numbers.
fn library() -> LibraryCache {
    let nocturnes = Album {
        id: "album-nocturnes".into(),
        name: "Nocturnes".into(),
        cover_url: "/covers/nocturnes.jpg".into(),
        first_release_date: "2019".into(),
        songs: vec![
            song("noc-2", "Nocturne No. 2", "Clara Keys", 2, true),
            song("noc-1", "Nocturne No. 1", "Clara Keys", 1, true),
            song("noc-3", "Nocturne No. 3", "Clara Keys", 3, true),
        ],
        ..Album::default()
    };
    let etudes = Album {
        id: "album-etudes".into(),
        name: "Etudes".into(),
        cover_url: "/covers/etudes.jpg".into(),
        first_release_date: "2021".into(),
        songs: vec![song("etu-1", "Etude in Blue", "Clara Keys", 1, true)],
        ..Album::default()
    };
    let neon = Album {
        id: "album-neon".into(),
        name: "The Neon Drive".into(),
        cover_url: "/covers/neon.jpg".into(),
        first_release_date: "2020".into(),
        songs: vec![
            song("neo-1", "Night Drive", "Synth Club", 1, false),
            song("neo-2", "Arcade Lights", "Synth Club", 2, false),
        ],
        ..Album::default()
    };
    let artists = vec![
        Artist {
            id: "artist-clara".into(),
            name: "Clara Keys".into(),
            albums: vec![nocturnes, etudes],
            ..Artist::default()
        },
        Artist {
            id: "artist-synth".into(),
            name: "Synth Club".into(),
            albums: vec![neon],
            ..Artist::default()
        },
    ];
    let mut song_map = HashMap::new();
    let mut song_positions = HashMap::new();
    let mut album_positions = HashMap::new();
    let mut songs_flat = Vec::new();
    let mut songs_by_artist: HashMap<String, Vec<u32>> = HashMap::new();
    for (artist_index, artist) in artists.iter().enumerate() {
        for (album_index, album) in artist.albums.iter().enumerate() {
            album_positions.insert(album.id.clone(), (artist_index as u32, album_index as u32));
            for (song_index, song) in album.songs.iter().enumerate() {
                song_map.insert(song.id.clone(), (artist.id.clone(), album.id.clone()));
                song_positions.insert(
                    song.id.clone(),
                    (artist_index as u32, album_index as u32, song_index as u32),
                );
                songs_by_artist
                    .entry(artist.id.clone())
                    .or_default()
                    .push(songs_flat.len() as u32);
                songs_flat.push(song.id.clone());
            }
        }
    }
    let index_of = |id: &str| songs_flat.iter().position(|song| song == id).unwrap() as u32;
    let songs_by_genre = HashMap::from([
        (
            "Classical".to_string(),
            ["noc-1", "noc-2", "noc-3", "etu-1"]
                .iter()
                .map(|id| index_of(id))
                .collect(),
        ),
        (
            "Synthwave".to_string(),
            ["neo-1", "neo-2"].iter().map(|id| index_of(id)).collect(),
        ),
    ]);
    LibraryCache {
        search_index: SearchIndex::build(&artists).expect("search index"),
        artists: Arc::new(artists),
        song_map,
        album_genres: HashMap::from([
            ("album-nocturnes".to_string(), vec!["Classical".to_string()]),
            ("album-etudes".to_string(), vec!["Classical".to_string()]),
            ("album-neon".to_string(), vec!["Synthwave".to_string()]),
        ]),
        artist_positions: HashMap::from([
            ("artist-clara".to_string(), 0),
            ("artist-synth".to_string(), 1),
        ]),
        album_positions,
        song_positions,
        songs_flat,
        songs_by_artist,
        songs_by_genre,
        image_paths: Default::default(),
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

fn migrated_pool() -> DbPool {
    use diesel::r2d2::{ConnectionManager, Pool};
    let pool = Arc::new(
        Pool::builder()
            .max_size(1)
            .build(ConnectionManager::<diesel::sqlite::SqliteConnection>::new(
                ":memory:",
            ))
            .expect("test pool"),
    );
    let mut connection = pool.get().expect("connection");
    connection
        .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
        .expect("migrations");
    // Fixtures reference no user or catalog rows.
    diesel::sql_query("PRAGMA foreign_keys = OFF")
        .execute(&mut connection)
        .expect("pragma");
    diesel::sql_query(
        "INSERT INTO playlist (id, name, is_public) VALUES (41, 'Late Night', false)",
    )
    .execute(&mut connection)
    .expect("playlist");
    diesel::sql_query("INSERT INTO _playlist_to_user (a, b) VALUES (41, 3)")
        .execute(&mut connection)
        .expect("playlist owner");
    for (position, song) in ["neo-2", "noc-3", "neo-1"].iter().enumerate() {
        diesel::sql_query(format!(
            "INSERT INTO _playlist_to_song (a, b, position) VALUES (41, '{song}', {position})"
        ))
        .execute(&mut connection)
        .expect("playlist song");
    }
    drop(connection);
    pool
}

fn registration(song_ids: Option<Vec<String>>) -> ClientRegistration {
    ClientRegistration {
        kind: ZoneKind::Desktop,
        name: "Listening Room".into(),
        device: "Mujin Music for Linux".into(),
        status: ZoneStatus::Playing,
        song_ids,
        current_position: 0,
        position_ms: 10_000,
        duration_ms: 200_000,
        volume: 40,
        muted: false,
        acknowledged_seq: 0,
        repeat: RepeatMode::Off,
        shuffle: false,
    }
}

/// Commands the client zone has been sent since it last checked in; the
/// heartbeat acknowledges them so the next call sees only new ones.
fn pending(service: &ZoneService) -> Vec<ZoneCommand> {
    pending_at(service, 0)
}

/// [`pending`] from a client playing queue position `current_position`.
fn pending_at(service: &ZoneService, current_position: usize) -> Vec<ZoneCommand> {
    let heartbeat = || ClientRegistration {
        current_position,
        ..registration(None)
    };
    let commands = service
        .register_client(USER, DEVICE, heartbeat())
        .expect("heartbeat")
        .commands;
    if let Some(last) = commands.iter().map(|pending| pending.seq).max() {
        service
            .register_client(
                USER,
                DEVICE,
                ClientRegistration {
                    acknowledged_seq: last,
                    ..heartbeat()
                },
            )
            .expect("acknowledge");
    }
    commands
        .into_iter()
        .map(|pending| pending.command)
        .collect()
}

macro_rules! app {
    ($service:expr, $lifecycle:expr, $pool:expr) => {
        actix_test::init_service(
            App::new()
                .app_data($service.clone())
                .app_data($lifecycle.clone())
                .app_data($pool.clone())
                .wrap_fn(|request, next| {
                    request.extensions_mut().insert(claims());
                    next.call(request)
                })
                .configure(crate::api::zones::configure)
                .configure(super::configure),
        )
        .await
    };
}

async fn setup() -> (
    web::Data<ZoneService>,
    web::Data<LibraryLifecycle>,
    web::Data<DbPool>,
) {
    let service = web::Data::new(ZoneService::new(None, None));
    let lifecycle = web::Data::new(LibraryLifecycle::new());
    lifecycle.set_ready(library()).await;
    let pool = web::Data::new(migrated_pool());
    service
        .register_client(
            USER,
            DEVICE,
            registration(Some(vec!["neo-1".into(), "neo-2".into()])),
        )
        .expect("register");
    (service, lifecycle, pool)
}

#[test]
fn item_ids_round_trip_and_reject_junk() {
    for id in [
        "root",
        "albums",
        "albums/C",
        "artists/#",
        "playlists",
        "genres",
        "radio",
        "liked",
        "album:x",
        "artist:y",
        "playlist:41",
        "genre:Drum & Bass",
        "song:z",
        "radio:library",
        "radio:genre:Jazz",
    ] {
        let parsed = ItemId::parse(id).unwrap_or_else(|| panic!("{id} parses"));
        let expected = match id {
            "albums/C" | "artists/#" => id.split('/').next().unwrap().to_string(),
            _ => id.to_string(),
        };
        assert_eq!(item_id_string(&parsed), expected);
    }
    for junk in [
        "",
        "album:",
        "playlist:abc",
        "albums/cd",
        "albums/c",
        "nonsense",
        &"a".repeat(600),
    ] {
        assert_eq!(ItemId::parse(junk), None, "{junk:?} is rejected");
    }
}

#[test]
fn quality_labels_lead_with_the_lossless_format() {
    let hires = AudioProperties {
        codec: Some("flac".into()),
        container: Some("flac".into()),
        sample_rate: Some(96_000),
        bit_depth: Some(24),
        channels: Some(2),
        bitrate_kbps: None,
        lossless: Some(true),
    };
    assert_eq!(quality_label(&hires).as_deref(), Some("FLAC 24/96"));
    let cd = AudioProperties {
        codec: Some("alac".into()),
        sample_rate: Some(44_100),
        bit_depth: Some(16),
        ..hires.clone()
    };
    assert_eq!(quality_label(&cd).as_deref(), Some("ALAC 16/44.1"));
    let lossy = AudioProperties {
        codec: Some("mp3".into()),
        bitrate_kbps: Some(320),
        lossless: Some(false),
        ..hires.clone()
    };
    assert_eq!(quality_label(&lossy).as_deref(), Some("MP3 320"));
    assert_eq!(quality_label(&AudioProperties::default()), None);
}

#[test]
fn long_lists_split_into_letter_buckets() {
    let items: Vec<(String, BrowseItem)> = (0..(BUCKET_THRESHOLD + 5))
        .map(|index| {
            let name = match index % 3 {
                0 => format!("Alpha {index}"),
                1 => format!("The Beta {index}"),
                _ => format!("42 Gamma {index}"),
            };
            (
                name.clone(),
                directory(&format!("album:{index}"), &name, true),
            )
        })
        .collect();
    let buckets = bucketed("albums", items.clone(), None);
    let titles: Vec<&str> = buckets.iter().map(|item| item.title.as_str()).collect();
    // "The Beta" files under B, numbers go last.
    assert_eq!(titles, vec!["A", "B", "#"]);
    assert_eq!(buckets[0].id, "albums/A");
    let b = bucketed("albums", items, Some('B'));
    assert!(b.iter().all(|item| item.title.starts_with("The Beta")));
}

#[actix_web::test]
async fn browse_walks_the_library_with_artwork_and_quality() {
    let (service, lifecycle, pool) = setup().await;
    let app = app!(service, lifecycle, pool);

    let root: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get().uri("/browse").to_request(),
    )
    .await;
    let ids: Vec<&str> = root["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["albums", "artists", "playlists", "genres", "liked", "radio"]
    );

    let albums: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=albums")
            .to_request(),
    )
    .await;
    let titles: Vec<&str> = albums["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    // "The Neon Drive" sorts under N.
    assert_eq!(titles, vec!["Etudes", "The Neon Drive", "Nocturnes"]);
    assert_eq!(albums["children"][2]["quality"], "FLAC 24/96");
    assert_eq!(albums["children"][2]["subtitle"], "Clara Keys");
    assert_eq!(albums["children"][2]["image"], "/covers/nocturnes.jpg");

    let album: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=album:album-nocturnes")
            .to_request(),
    )
    .await;
    assert_eq!(album["item"]["title"], "Nocturnes");
    let tracks: Vec<&str> = album["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(tracks, vec!["song:noc-1", "song:noc-2", "song:noc-3"]);
    assert_eq!(album["children"][0]["duration_ms"], 200_000);

    let artist: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=artist:artist-clara")
            .to_request(),
    )
    .await;
    // Newest album first, then an artist station.
    assert_eq!(artist["children"][0]["title"], "Etudes");
    assert_eq!(artist["children"][2]["id"], "radio:artist:artist-clara");
    assert_eq!(artist["item"]["image"], "/covers/nocturnes.jpg");

    let genres: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=genres")
            .to_request(),
    )
    .await;
    assert_eq!(genres["children"][0]["id"], "genre:Classical");
    assert_eq!(genres["children"][0]["subtitle"], "4 songs");

    let genre: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=genre:Classical")
            .to_request(),
    )
    .await;
    assert_eq!(genre["children"][0]["id"], "radio:genre:Classical");
    assert_eq!(genre["children"].as_array().unwrap().len(), 3);

    let playlists: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=playlists")
            .to_request(),
    )
    .await;
    assert_eq!(playlists["children"][0]["id"], "playlist:41");
    assert_eq!(playlists["children"][0]["title"], "Late Night");
    assert_eq!(playlists["children"][0]["image"], "/covers/neon.jpg");

    let playlist: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=playlist:41")
            .to_request(),
    )
    .await;
    let order: Vec<&str> = playlist["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["song:neo-2", "song:noc-3", "song:neo-1"]);

    let radio: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?item=radio")
            .to_request(),
    )
    .await;
    let stations: Vec<&str> = radio["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        stations,
        vec![
            "radio:library",
            "radio:genre:Classical",
            "radio:genre:Synthwave"
        ]
    );

    let search: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::get()
            .uri("/browse?query=nocturne")
            .to_request(),
    )
    .await;
    let kinds: Vec<&str> = search["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"album"));
    assert!(kinds.contains(&"song"));
    let first_song = kinds.iter().position(|kind| *kind == "song").unwrap();
    assert!(kinds[..first_song].iter().all(|kind| *kind != "song"));

    for (uri, status) in [
        ("/browse?item=album:missing", 404),
        ("/browse?item=what", 400),
        ("/browse?item=radio:library", 400),
        ("/browse?query=%20", 400),
    ] {
        let response =
            actix_test::call_service(&app, actix_test::TestRequest::get().uri(uri).to_request())
                .await;
        assert_eq!(response.status(), status, "{uri}");
    }
}

#[actix_web::test]
async fn play_resolves_items_and_searches_into_zone_commands() {
    let (service, lifecycle, pool) = setup().await;
    let app = app!(service, lifecycle, pool);
    pending(&service);

    let played: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "album:album-nocturnes"}))
            .to_request(),
    )
    .await;
    assert_eq!(played["title"], "Nocturnes");
    assert_eq!(played["songs"], 3);
    assert_eq!(
        pending(&service),
        vec![ZoneCommand::Load {
            song_ids: vec!["noc-1".into(), "noc-2".into(), "noc-3".into()],
            current_position: 0,
            position_ms: 0,
            play: true,
        }]
    );

    // Artist: oldest album first, each album in track order.
    actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "artist:artist-clara", "action": "add"}))
            .to_request(),
    )
    .await;
    assert_eq!(
        pending(&service),
        vec![ZoneCommand::AddToQueue {
            song_ids: vec![
                "noc-1".into(),
                "noc-2".into(),
                "noc-3".into(),
                "etu-1".into()
            ],
            next: false,
        }]
    );

    actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "playlist:41", "action": "next"}))
            .to_request(),
    )
    .await;
    assert_eq!(
        pending(&service),
        vec![ZoneCommand::AddToQueue {
            song_ids: vec!["neo-2".into(), "noc-3".into(), "neo-1".into()],
            next: true,
        }]
    );

    let shuffled: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "genre:Classical", "action": "shuffle"}))
            .to_request(),
    )
    .await;
    assert_eq!(shuffled["action"], "shuffle");
    match pending(&service).as_slice() {
        [ZoneCommand::Load { song_ids, .. }] => {
            let mut sorted = song_ids.clone();
            sorted.sort();
            assert_eq!(sorted, vec!["etu-1", "noc-1", "noc-2", "noc-3"]);
        }
        other => panic!("expected one load, got {other:?}"),
    }

    // Search: best match, optionally limited to a kind.
    let searched: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"query": "arcade lights"}))
            .to_request(),
    )
    .await;
    assert_eq!(searched["item_id"], "song:neo-2");
    let by_kind: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"query": "synth club", "kind": "artist"}))
            .to_request(),
    )
    .await;
    assert_eq!(by_kind["item_id"], "artist:artist-synth");
    let playlist: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"query": "late", "kind": "playlist"}))
            .to_request(),
    )
    .await;
    assert_eq!(playlist["item_id"], "playlist:41");
    let genre: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"query": "synthwave", "kind": "genre"}))
            .to_request(),
    )
    .await;
    assert_eq!(genre["item_id"], "genre:Synthwave");
    pending(&service);

    // A request that names nothing in the library is a clean "not
    // found" for that item; only a missing zone says zone_not_found.
    for (zone, body, status, code) in [
        (ZONE, serde_json::json!({}), 400, "item_or_query_required"),
        (
            ZONE,
            serde_json::json!({"item": "albums"}),
            400,
            "item_is_not_playable",
        ),
        (
            ZONE,
            serde_json::json!({"item": "album:nope"}),
            404,
            "item_not_found",
        ),
        (
            ZONE,
            serde_json::json!({"query": "zzzzqqq"}),
            404,
            "no_library_match",
        ),
        (
            ZONE,
            serde_json::json!({"query": "zzzzqqq", "kind": "playlist"}),
            404,
            "no_library_match",
        ),
        (
            ZONE,
            serde_json::json!({"query": "zzzzqqq", "kind": "genre"}),
            404,
            "no_library_match",
        ),
        (
            ZONE,
            serde_json::json!({"query": "x", "kind": "podcast"}),
            400,
            "invalid_search_kind",
        ),
        (
            "client:nobody-000000",
            serde_json::json!({"item": "song:neo-1"}),
            404,
            "zone_not_found",
        ),
    ] {
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri(&format!("/zones/{zone}/play"))
                .set_json(body.clone())
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status, "{zone} {body}");
        let error: serde_json::Value = actix_test::read_body_json(response).await;
        assert_eq!(error["error"], code, "{zone} {body}");
    }
    assert!(pending(&service).is_empty(), "failed requests send nothing");
}

#[actix_web::test]
async fn play_next_inserts_after_the_current_track_and_keeps_the_queue() {
    let (service, lifecycle, pool) = setup().await;
    let app = app!(service, lifecycle, pool);
    let queue: Vec<String> = ["noc-1", "noc-2", "noc-3", "neo-1", "neo-2"]
        .into_iter()
        .map(String::from)
        .collect();
    service
        .register_client(
            USER,
            DEVICE,
            ClientRegistration {
                current_position: 1,
                ..registration(Some(queue))
            },
        )
        .expect("five queued, the second playing");
    pending_at(&service, 1);

    let response = actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "song:etu-1", "action": "next"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let zone = service.zone(USER, ZONE, None).expect("zone");
    assert_eq!(
        zone.queue.song_ids,
        vec!["noc-1", "noc-2", "etu-1", "noc-3", "neo-1", "neo-2"],
        "inserted after the playing track, nothing dropped"
    );
    assert_eq!(zone.queue.current_position, 1);
    assert_eq!(
        pending_at(&service, 1),
        vec![ZoneCommand::AddToQueue {
            song_ids: vec!["etu-1".into()],
            next: true,
        }]
    );
}

#[actix_web::test]
async fn radio_starts_from_a_seed_and_never_comes_back_empty() {
    let (service, lifecycle, pool) = setup().await;
    let app = app!(service, lifecycle, pool);
    pending(&service);

    let station: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/radio"))
            .set_json(serde_json::json!({"item": "song:noc-2"}))
            .to_request(),
    )
    .await;
    assert_eq!(station["action"], "radio");
    assert_eq!(station["title"], "Nocturne No. 2 radio");
    match pending(&service).as_slice() {
        [ZoneCommand::Load { song_ids, play, .. }] => {
            assert!(*play);
            assert_eq!(song_ids[0], "noc-2", "the seed plays first");
            // A fresh profile still fills the station from the library.
            assert_eq!(song_ids.len(), 6);
            let unique: HashSet<_> = song_ids.iter().collect();
            assert_eq!(unique.len(), song_ids.len());
        }
        other => panic!("expected a radio load, got {other:?}"),
    }

    // A genre station opens on that genre.
    actix_test::call_service(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "radio:genre:Synthwave"}))
            .to_request(),
    )
    .await;
    match pending(&service).as_slice() {
        [ZoneCommand::Load { song_ids, .. }] => {
            assert!(song_ids[0].starts_with("neo-"));
        }
        other => panic!("expected a genre radio load, got {other:?}"),
    }

    // The artist station offered on artist pages, and library radio with
    // no body at all.
    let artist: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/play"))
            .set_json(serde_json::json!({"item": "radio:artist:artist-synth"}))
            .to_request(),
    )
    .await;
    assert_eq!(artist["title"], "Synth Club radio");
    pending(&service);
    let library: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/radio"))
            .to_request(),
    )
    .await;
    assert_eq!(library["item_id"], "radio:library");
    assert_eq!(library["songs"], 6);
}

#[actix_web::test]
async fn thumbs_teach_radio_and_thumbs_down_skips_the_playing_track() {
    let (service, lifecycle, pool) = setup().await;
    let app = app!(service, lifecycle, pool);
    pending(&service);

    let up: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/rating"))
            .set_json(serde_json::json!({"rating": "up"}))
            .to_request(),
    )
    .await;
    assert_eq!(up["song_id"], "neo-1");
    assert_eq!(up["skipped"], false);
    assert!(pending(&service).is_empty());

    let down: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/rating"))
            .set_json(serde_json::json!({"rating": "down"}))
            .to_request(),
    )
    .await;
    assert_eq!(down["rating"], "down");
    assert_eq!(down["skipped"], true);
    assert_eq!(pending(&service), vec![ZoneCommand::Next]);

    // Rating another song records it without touching playback.
    let other: serde_json::Value = actix_test::call_and_read_body_json(
        &app,
        actix_test::TestRequest::post()
            .uri(&format!("/zones/{ZONE}/rating"))
            .set_json(serde_json::json!({"rating": "down", "song_id": "noc-1"}))
            .to_request(),
    )
    .await;
    assert_eq!(other["skipped"], false);
    assert!(pending(&service).is_empty());

    #[derive(diesel::QueryableByName)]
    struct Preference {
        #[diesel(sql_type = diesel::sql_types::Text)]
        song_id: String,
        #[diesel(sql_type = diesel::sql_types::Double)]
        preference_score: f64,
    }
    let mut connection = pool.get().expect("connection");
    let mut preferences: Vec<Preference> = diesel::sql_query(
        "SELECT song_id, preference_score FROM user_track_preference
         WHERE user_id = 3 ORDER BY song_id",
    )
    .load(&mut connection)
    .expect("preferences");
    preferences.sort_by(|left, right| left.song_id.cmp(&right.song_id));
    let scores: Vec<(&str, f64)> = preferences
        .iter()
        .map(|row| (row.song_id.as_str(), row.preference_score))
        .collect();
    assert_eq!(scores, vec![("neo-1", -10.0), ("noc-1", -20.0)]);
    drop(connection);

    for (body, status) in [
        (serde_json::json!({"rating": "sideways"}), 400),
        (
            serde_json::json!({"rating": "up", "song_id": "missing"}),
            400,
        ),
    ] {
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::post()
                .uri(&format!("/zones/{ZONE}/rating"))
                .set_json(body.clone())
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status, "{body}");
    }
}
