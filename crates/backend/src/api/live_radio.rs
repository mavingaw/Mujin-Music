//! Live internet radio API, mounted under `/live-radio`:
//!
//! - `GET    /home`                Live Radio landing: My Live Radio, recently
//!   played, popular, lossless and trending stations, categories
//! - `GET    /categories`          curated genres and collections
//! - `GET    /stations`            directory search (name, tag, country_code,
//!   language, codec, min_bitrate, lossless, order, offset, limit)
//! - `GET    /stations/{id}`       one station, what it is playing, similar
//! - `GET    /facets/{kind}`       tags, countries, languages or codecs
//! - `GET    /now-playing?ids=`    titles and formats for several stations
//! - `POST   /check`               opens a URL and reports what it carries
//! - `GET    /my`                  My Live Radio
//! - `POST   /my`                  adds a station by URL
//! - `PUT    /my/{id}`             saves a directory station
//! - `PATCH  /my/{id}`             edits a custom station
//! - `DELETE /my/{id}`             removes a station
//! - `PUT    /my-order`            reorders My Live Radio
//! - `GET    /recent`, `DELETE /recent`
//!
//! Stations stream through `/media/songs/radio:<id>/stream` and describe
//! their signal path at `/media/songs/radio:<id>/signal-path`; logos come
//! from `/live-radio/stations/{id}/logo` (no sign-in, so `<img>` works
//! everywhere; only stations Parson already knows are served).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use actix_web::http::header;
use actix_web::{HttpMessage, HttpRequest, HttpResponse, delete, get, patch, post, put, web};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::auth::{Claims, authenticated_user_id};
use crate::api::error::{bad_request, forbidden, json_error, not_found, service_unavailable};
use crate::live_radio::directory::{Facet, FacetKind, Order, SearchQuery};
use crate::live_radio::relay::{Destination, RelayError};
use crate::live_radio::station::{
    Station, StationSource, StationView, clean, station_id_from_song, valid_station_id,
};
use crate::live_radio::{LiveRadio, NowPlaying, StreamCheck, store};
use crate::persistence::connection::DbPool;
use crate::signal_path::StreamTarget;

fn service(data: Option<web::Data<LiveRadio>>) -> Arc<LiveRadio> {
    data.map(|data| data.into_inner())
        .unwrap_or_else(crate::live_radio::global)
}

fn is_admin(request: &HttpRequest) -> bool {
    request
        .extensions()
        .get::<Claims>()
        .is_some_and(|claims| claims.role == "admin")
}

async fn db<T: Send + 'static>(
    pool: &DbPool,
    work: impl FnOnce(&mut diesel::SqliteConnection) -> Result<T, String> + Send + 'static,
) -> Result<T, HttpResponse> {
    let pool = pool.clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        work(&mut connection)
    })
    .await
    {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::error!(%error, "live radio storage failed");
            Err(HttpResponse::InternalServerError().finish())
        }
        Err(error) => {
            tracing::error!(%error, "live radio storage task failed");
            Err(HttpResponse::InternalServerError().finish())
        }
    }
}

async fn saved_ids(pool: &DbPool, user_id: i32) -> HashSet<String> {
    db(pool, move |connection| {
        store::saved_ids(connection, user_id).map_err(|error| error.to_string())
    })
    .await
    .unwrap_or_default()
}

fn views(radio: &LiveRadio, stations: &[Station], saved: &HashSet<String>) -> Vec<StationView> {
    stations
        .iter()
        .map(|station| radio.view(station, saved.contains(&station.id)))
        .collect()
}

fn relay_error(error: &RelayError) -> HttpResponse {
    let status = if error.is_client_error() {
        actix_web::http::StatusCode::UNPROCESSABLE_ENTITY
    } else {
        actix_web::http::StatusCode::BAD_GATEWAY
    };
    json_error(status, "stream_failed", error.to_string(), error.code())
}

fn directory_error(error: &crate::live_radio::directory::DirectoryError) -> HttpResponse {
    service_unavailable(error.to_string(), "radio_directory_unavailable")
}

/// A curated way into the directory.
#[derive(Serialize, Clone)]
struct Category {
    id: &'static str,
    label: &'static str,
    /// `collections`, `music` or `talk`.
    group: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<&'static str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    lossless: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<Order>,
    /// Two colours for the tile's gradient.
    colors: [&'static str; 2],
}

const fn genre(id: &'static str, label: &'static str, colors: [&'static str; 2]) -> Category {
    Category {
        id,
        label,
        group: "music",
        tag: Some(id),
        lossless: false,
        min_bitrate: None,
        order: None,
        colors,
    }
}

const fn talk(id: &'static str, label: &'static str, colors: [&'static str; 2]) -> Category {
    Category {
        group: "talk",
        ..genre(id, label, colors)
    }
}

const CATEGORIES: [Category; 27] = [
    Category {
        id: "lossless",
        label: "Lossless",
        group: "collections",
        tag: None,
        lossless: true,
        min_bitrate: None,
        order: Some(Order::Votes),
        colors: ["#7c3aed", "#312e81"],
    },
    Category {
        id: "hifi",
        label: "Hi-fi 256 kbps+",
        group: "collections",
        tag: None,
        lossless: false,
        min_bitrate: Some(256),
        order: Some(Order::Popular),
        colors: ["#0ea5e9", "#1e3a8a"],
    },
    Category {
        id: "trending",
        label: "Trending now",
        group: "collections",
        tag: None,
        lossless: false,
        min_bitrate: None,
        order: Some(Order::Trending),
        colors: ["#f97316", "#9f1239"],
    },
    genre("jazz", "Jazz", ["#b45309", "#422006"]),
    genre("classical", "Classical", ["#a16207", "#3f2d0c"]),
    genre("electronic", "Electronic", ["#06b6d4", "#312e81"]),
    genre("ambient", "Ambient", ["#14b8a6", "#134e4a"]),
    genre("rock", "Rock", ["#dc2626", "#450a0a"]),
    genre("pop", "Pop", ["#ec4899", "#581c87"]),
    genre("hip hop", "Hip-Hop", ["#eab308", "#3f3f46"]),
    genre("soul", "Soul & R&B", ["#c026d3", "#4a044e"]),
    genre("blues", "Blues", ["#2563eb", "#172554"]),
    genre("country", "Country", ["#d97706", "#451a03"]),
    genre("folk", "Folk", ["#65a30d", "#1a2e05"]),
    genre("reggae", "Reggae", ["#16a34a", "#713f12"]),
    genre("latin", "Latin", ["#f43f5e", "#7c2d12"]),
    genre("world", "World", ["#0d9488", "#7c2d12"]),
    genre("metal", "Metal", ["#52525b", "#09090b"]),
    genre("indie", "Indie", ["#8b5cf6", "#1e1b4b"]),
    genre("chillout", "Chillout", ["#38bdf8", "#0c4a6e"]),
    genre("lounge", "Lounge", ["#a855f7", "#3b0764"]),
    genre("house", "House", ["#6366f1", "#1e1b4b"]),
    genre("80s", "80s", ["#f472b6", "#1e40af"]),
    genre("oldies", "Oldies", ["#ca8a04", "#431407"]),
    talk("news", "News", ["#64748b", "#0f172a"]),
    talk("talk", "Talk", ["#78716c", "#1c1917"]),
    talk("sports", "Sports", ["#22c55e", "#052e16"]),
];

#[get("/categories")]
async fn categories() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "items": CATEGORIES }))
}

#[derive(Deserialize)]
struct HomeQuery {
    /// The listener's country (ISO 3166-1), for local stations.
    #[serde(default)]
    country: Option<String>,
}

#[derive(Serialize)]
struct Home {
    my: Vec<StationView>,
    recent: Vec<StationView>,
    popular: Vec<StationView>,
    lossless: Vec<StationView>,
    trending: Vec<StationView>,
    local: Vec<StationView>,
    categories: &'static [Category],
    /// Set when the directory could not be reached; saved stations still work.
    directory_error: Option<String>,
}

#[get("/home")]
async fn home(
    request: HttpRequest,
    query: web::Query<HomeQuery>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let radio = service(radio);
    let (my, recent) = match db(&pool, move |connection| {
        Ok((
            store::list_saved(connection, user_id).map_err(|error| error.to_string())?,
            store::list_recent(connection, user_id).map_err(|error| error.to_string())?,
        ))
    })
    .await
    {
        Ok(lists) => lists,
        Err(response) => return response,
    };
    let page = |order: Order, limit: u32| SearchQuery {
        order,
        limit: Some(limit),
        ..SearchQuery::default()
    };
    let local_query = query
        .country
        .as_deref()
        .and_then(|code| clean(Some(code), 3))
        .filter(|code| code.len() == 2)
        .map(|code| SearchQuery {
            country_code: Some(code),
            ..page(Order::Popular, 12)
        });
    let directory = radio.directory();
    let lossless_query = SearchQuery {
        lossless: true,
        ..page(Order::Votes, 12)
    };
    let popular_query = page(Order::Popular, 18);
    let trending_query = page(Order::Trending, 12);
    let (popular, lossless, trending, local) = futures::join!(
        directory.search(&popular_query),
        radio.search(&lossless_query),
        directory.search(&trending_query),
        async {
            match &local_query {
                Some(query) => directory.search(query).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let directory_error = [
        popular.as_ref().err(),
        lossless.as_ref().err(),
        trending.as_ref().err(),
    ]
    .into_iter()
    .flatten()
    .next()
    .map(ToString::to_string);
    let popular = popular.unwrap_or_default();
    let lossless = lossless.unwrap_or_default();
    let trending = trending.unwrap_or_default();
    let local = local.ok().flatten().unwrap_or_default();
    for list in [&my, &recent, &popular, &lossless, &trending, &local] {
        radio.remember(list);
    }
    let saved: HashSet<String> = my.iter().map(|station| station.id.clone()).collect();
    HttpResponse::Ok().json(Home {
        my: views(&radio, &my, &saved),
        recent: views(&radio, &recent, &saved),
        popular: views(&radio, &popular, &saved),
        lossless: views(&radio, &lossless, &saved),
        trending: views(&radio, &trending, &saved),
        local: views(&radio, &local, &saved),
        categories: &CATEGORIES,
        directory_error,
    })
}

#[derive(Serialize)]
struct StationPage {
    items: Vec<StationView>,
    offset: u32,
    limit: u32,
    has_more: bool,
}

#[get("/stations")]
async fn search(
    request: HttpRequest,
    query: web::Query<SearchQuery>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let radio = service(radio);
    let query = query.into_inner();
    let limit = query
        .limit
        .unwrap_or(40)
        .clamp(1, crate::live_radio::directory::MAX_PAGE);
    match radio.search(&query).await {
        Ok(stations) => {
            radio.remember(&stations);
            let saved = saved_ids(&pool, user_id).await;
            HttpResponse::Ok().json(StationPage {
                has_more: stations.len() as u32 >= limit,
                items: views(&radio, &stations, &saved),
                offset: query.offset,
                limit,
            })
        }
        Err(error) => directory_error(&error),
    }
}

#[derive(Serialize)]
struct StationDetail {
    station: StationView,
    now_playing: NowPlaying,
    /// Stations sharing this one's main genre.
    similar: Vec<StationView>,
}

#[get("/stations/{id}")]
async fn station_detail(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let radio = service(radio);
    let Some(station) = radio
        .find(Some(pool.get_ref()), &id, Some(user_id), true)
        .await
    else {
        return not_found("Station not found.", "station_not_found");
    };
    let saved = saved_ids(&pool, user_id).await;
    let similar = match station.tags.first() {
        Some(tag) => radio
            .directory()
            .search(&SearchQuery {
                tag: Some(tag.clone()),
                limit: Some(13),
                ..SearchQuery::default()
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|other| other.id != station.id)
            .take(12)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    radio.remember(&similar);
    HttpResponse::Ok().json(StationDetail {
        now_playing: radio.now_playing(&station.id),
        station: radio.view(&station, saved.contains(&station.id)),
        similar: views(&radio, &similar, &saved),
    })
}

/// Opens a station briefly and reads its real format from the audio.
#[post("/stations/{id}/verify")]
async fn verify_station(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let radio = service(radio);
    let Some(station) = radio
        .find(Some(pool.get_ref()), &id, Some(user_id), true)
        .await
    else {
        return not_found("Station not found.", "station_not_found");
    };
    if let Err(error) = radio.verify(&station).await {
        return relay_error(&error);
    }
    let saved = saved_ids(&pool, user_id).await;
    HttpResponse::Ok().json(radio.view(&station, saved.contains(&station.id)))
}

#[derive(Deserialize)]
struct FacetQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[get("/facets/{kind}")]
async fn facets(
    kind: web::Path<String>,
    query: web::Query<FacetQuery>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let Some(kind) = FacetKind::parse(&kind) else {
        return not_found("Unknown facet.", "unknown_facet");
    };
    let limit = query.limit.unwrap_or(200);
    match service(radio).directory().facets(kind, limit).await {
        Ok(items) => HttpResponse::Ok().json(serde_json::json!({ "items": items as Vec<Facet> })),
        Err(error) => directory_error(&error),
    }
}

#[derive(Deserialize)]
struct NowPlayingQuery {
    ids: String,
}

#[get("/now-playing")]
async fn now_playing(
    query: web::Query<NowPlayingQuery>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let radio = service(radio);
    let items: HashMap<String, NowPlaying> = query
        .ids
        .split(',')
        .map(str::trim)
        .filter(|id| valid_station_id(id))
        .take(50)
        .map(|id| (id.to_string(), radio.now_playing(id)))
        .collect();
    HttpResponse::Ok().json(serde_json::json!({ "items": items }))
}

#[derive(Deserialize)]
struct CheckRequest {
    url: String,
    #[serde(default)]
    allow_lan: bool,
}

fn lan_forbidden() -> HttpResponse {
    forbidden(
        "Only an administrator can add streams on the local network.",
        "lan_streams_admin_only",
    )
}

#[post("/check")]
async fn check_stream(
    request: HttpRequest,
    body: web::Json<CheckRequest>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    if let Err(response) = authenticated_user_id(&request) {
        return response;
    }
    if body.allow_lan && !is_admin(&request) {
        return lan_forbidden();
    }
    match service(radio).check(&body.url, body.allow_lan).await {
        Ok(check) => HttpResponse::Ok().json(check),
        Err(error) => relay_error(&error),
    }
}

#[get("/my")]
async fn my_stations(
    request: HttpRequest,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match db(&pool, move |connection| {
        store::list_saved(connection, user_id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(stations) => {
            let radio = service(radio);
            radio.remember(&stations);
            let saved = stations.iter().map(|station| station.id.clone()).collect();
            HttpResponse::Ok()
                .json(serde_json::json!({ "items": views(&radio, &stations, &saved) }))
        }
        Err(response) => response,
    }
}

async fn save_station(pool: &DbPool, user_id: i32, station: Station) -> HttpResponse {
    let stored = station.clone();
    match db(pool, move |connection| {
        Ok(store::save(connection, user_id, &stored))
    })
    .await
    {
        Ok(Ok(())) => HttpResponse::Ok().json(StationView::new(&station, true)),
        Ok(Err(store::SaveError::TooMany)) => bad_request(
            "My Live Radio is full (1,000 stations).",
            "too_many_stations",
        ),
        Ok(Err(store::SaveError::Database(error))) => {
            tracing::error!(%error, "could not save a station");
            HttpResponse::InternalServerError().finish()
        }
        Err(response) => response,
    }
}

#[put("/my/{id}")]
async fn save_directory_station(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(station) = service(radio)
        .find(Some(pool.get_ref()), &id, Some(user_id), true)
        .await
    else {
        return not_found("Station not found.", "station_not_found");
    };
    save_station(&pool, user_id, station).await
}

#[derive(Deserialize)]
struct CustomStation {
    url: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    logo_url: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    allow_lan: bool,
}

fn http_url(value: Option<&str>) -> Option<String> {
    let value = clean(value, 2048)?;
    crate::live_radio::guard::parse_url(&value)
        .ok()
        .map(|url| url.to_string())
}

fn clean_tags(tags: Option<&[String]>, fallback: Option<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    let source: Vec<String> = match tags {
        Some(tags) => tags.to_vec(),
        None => fallback
            .map(|genre| genre.split([',', '/', ';']).map(str::to_string).collect())
            .unwrap_or_default(),
    };
    source
        .into_iter()
        .filter_map(|tag| clean(Some(&tag.to_lowercase()), 40))
        .filter(|tag| seen.insert(tag.clone()))
        .take(12)
        .collect()
}

fn custom_station(id: String, form: &CustomStation, check: &StreamCheck) -> Station {
    let host = reqwest::Url::parse(form.url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "Custom station".into());
    Station {
        id,
        source: StationSource::Custom,
        name: clean(form.name.as_deref(), 200)
            .or_else(|| clean(check.stream_name.as_deref(), 200))
            .unwrap_or(host),
        stream_url: form.url.trim().to_string(),
        homepage: http_url(form.homepage.as_deref()),
        favicon: http_url(form.logo_url.as_deref()),
        country: None,
        country_code: clean(form.country_code.as_deref(), 3)
            .filter(|code| code.len() == 2)
            .map(|code| code.to_ascii_uppercase()),
        region: None,
        languages: clean(form.language.as_deref(), 40)
            .map(|language| vec![language.to_lowercase()])
            .unwrap_or_default(),
        tags: clean_tags(form.tags.as_deref(), check.genre.as_deref()),
        codec: check.format.codec.clone(),
        bitrate_kbps: check.format.bitrate_kbps,
        hls: check.transport == crate::live_radio::relay::Transport::Hls,
        votes: None,
        clicks: None,
        click_trend: None,
        online: Some(true),
        description: clean(form.description.as_deref(), 1000),
        allow_lan: form.allow_lan,
    }
}

#[derive(Serialize)]
struct AddedStation {
    station: StationView,
    check: StreamCheck,
}

#[post("/my")]
async fn add_custom_station(
    request: HttpRequest,
    form: web::Json<CustomStation>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if form.allow_lan && !is_admin(&request) {
        return lan_forbidden();
    }
    let radio = service(radio);
    let check = match radio.check(&form.url, form.allow_lan).await {
        Ok(check) => check,
        Err(error) => return relay_error(&error),
    };
    let station = custom_station(uuid::Uuid::new_v4().to_string(), &form, &check);
    let response = save_station(&pool, user_id, station.clone()).await;
    if !response.status().is_success() {
        return response;
    }
    radio.remember(std::slice::from_ref(&station));
    HttpResponse::Ok().json(AddedStation {
        station: radio.view(&station, true),
        check,
    })
}

#[derive(Deserialize)]
struct StationEdit {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    logo_url: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    description: Option<String>,
}

#[patch("/my/{id}")]
async fn edit_custom_station(
    request: HttpRequest,
    id: web::Path<String>,
    edit: web::Json<StationEdit>,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    let lookup = id.clone();
    let station = match db(&pool, move |connection| {
        store::find_saved(connection, &lookup, Some(user_id)).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Some(station)) if station.source == StationSource::Custom => station,
        Ok(_) => return not_found("Custom station not found.", "station_not_found"),
        Err(response) => return response,
    };
    let radio = service(radio);
    let mut updated = station.clone();
    if let Some(url) = edit
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| *url != station.stream_url)
    {
        let check = match radio.check(url, station.allow_lan).await {
            Ok(check) => check,
            Err(error) => return relay_error(&error),
        };
        updated.stream_url = url.to_string();
        updated.codec = check.format.codec;
        updated.bitrate_kbps = check.format.bitrate_kbps;
        updated.hls = check.transport == crate::live_radio::relay::Transport::Hls;
    }
    if let Some(name) = clean(edit.name.as_deref(), 200) {
        updated.name = name;
    }
    if edit.homepage.is_some() {
        updated.homepage = http_url(edit.homepage.as_deref());
    }
    if edit.logo_url.is_some() {
        updated.favicon = http_url(edit.logo_url.as_deref());
    }
    if let Some(tags) = edit.tags.as_deref() {
        updated.tags = clean_tags(Some(tags), None);
    }
    if edit.description.is_some() {
        updated.description = clean(edit.description.as_deref(), 1000);
    }
    radio.remember(std::slice::from_ref(&updated));
    save_station(&pool, user_id, updated).await
}

#[delete("/my/{id}")]
async fn remove_station(
    request: HttpRequest,
    id: web::Path<String>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let id = id.into_inner();
    match db(&pool, move |connection| {
        store::remove(connection, user_id, &id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(true) => HttpResponse::NoContent().finish(),
        Ok(false) => not_found("Station not found.", "station_not_found"),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct OrderRequest {
    ids: Vec<String>,
}

#[put("/my-order")]
async fn reorder(
    request: HttpRequest,
    body: web::Json<OrderRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    if body.ids.len() > store::MAX_SAVED as usize || body.ids.iter().any(|id| !valid_station_id(id))
    {
        return bad_request("Invalid station order.", "invalid_station_order");
    }
    let ids = body.into_inner().ids;
    match db(&pool, move |connection| {
        store::reorder(connection, user_id, &ids).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(response) => response,
    }
}

#[get("/recent")]
async fn recent_stations(
    request: HttpRequest,
    pool: web::Data<DbPool>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let lists = db(&pool, move |connection| {
        Ok((
            store::list_recent(connection, user_id).map_err(|error| error.to_string())?,
            store::saved_ids(connection, user_id).map_err(|error| error.to_string())?,
        ))
    })
    .await;
    match lists {
        Ok((stations, saved)) => {
            let radio = service(radio);
            radio.remember(&stations);
            HttpResponse::Ok()
                .json(serde_json::json!({ "items": views(&radio, &stations, &saved) }))
        }
        Err(response) => response,
    }
}

#[delete("/recent")]
async fn clear_recent(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match db(&pool, move |connection| {
        store::clear_recent(connection, user_id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(()) => HttpResponse::NoContent().finish(),
        Err(response) => response,
    }
}

/// Station logos, relayed so listeners' addresses never reach station
/// hosts. Only stations Parson already knows are served.
pub async fn station_logo(
    id: web::Path<String>,
    pool: Option<web::Data<DbPool>>,
    radio: Option<web::Data<LiveRadio>>,
) -> HttpResponse {
    let radio = service(radio);
    let pool = pool.map(|pool| pool.get_ref().clone());
    let Some(station) = radio.find(pool.as_ref(), &id, None, false).await else {
        return HttpResponse::NotFound().finish();
    };
    match radio.logo(&station).await {
        Some((content_type, bytes)) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, content_type))
            .insert_header((header::CACHE_CONTROL, "public, max-age=86400"))
            .insert_header(("X-Content-Type-Options", "nosniff"))
            .insert_header((
                "Content-Security-Policy",
                "default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox",
            ))
            .body(bytes),
        None => HttpResponse::NotFound()
            .insert_header((header::CACHE_CONTROL, "public, max-age=3600"))
            .finish(),
    }
}

/// The codecs a player says it decodes (`accept=flac,mp3,...`).
fn accepted_codecs(query: &str) -> Option<Vec<String>> {
    let params = web::Query::<HashMap<String, String>>::from_query(query).ok()?;
    let accept = params.get("accept")?;
    let mut list: Vec<String> = Vec::new();
    for codec in accept
        .split(',')
        .map(|codec| codec.trim().to_ascii_lowercase())
    {
        match codec.as_str() {
            "" => {}
            // A bare container name covers the codecs it carries.
            "ogg" => list.extend(["vorbis".to_string(), "opus".to_string()]),
            "m4a" | "mp4" => list.push("aac".into()),
            _ => list.push(codec),
        }
    }
    (!list.is_empty()).then_some(list)
}

/// Streams a station to a Parson app (the `/media/songs/radio:<id>/stream`
/// route). Records the play and reports a directory click.
pub async fn stream_station(request: &HttpRequest, song_id: &str) -> HttpResponse {
    let user_id = match authenticated_user_id(request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(station_id) = station_id_from_song(song_id) else {
        return not_found("Station not found.", "station_not_found");
    };
    let radio = service(request.app_data::<web::Data<LiveRadio>>().cloned());
    let pool = request
        .app_data::<web::Data<DbPool>>()
        .map(|pool| pool.get_ref().clone());
    let Some(station) = radio
        .find(pool.as_ref(), station_id, Some(user_id), true)
        .await
    else {
        return not_found("Station not found.", "station_not_found");
    };
    let destination = Destination::Player {
        accepts: accepted_codecs(request.query_string()),
    };
    let relayed = match radio.open(&station, &destination).await {
        Ok(relayed) => relayed,
        Err(error) => return relay_error(&error),
    };
    radio.report_click(&station);
    if let Some(pool) = pool {
        let played = station.clone();
        actix_web::rt::spawn(async move {
            let _ = web::block(move || {
                let mut connection = pool.get().map_err(|error| error.to_string())?;
                store::record_recent(&mut connection, user_id, &played)
                    .map_err(|error| error.to_string())
            })
            .await;
        });
    }
    live_response(relayed)
}

/// A live stream response: no length, no ranges, never cached.
pub fn live_response(relayed: crate::live_radio::Relayed) -> HttpResponse {
    let label = crate::live_radio::station::classify(
        relayed.format.codec.as_deref(),
        relayed.format.bitrate_kbps,
    )
    .label;
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, relayed.content_type))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .insert_header((header::ACCEPT_RANGES, "none"))
        .insert_header(("X-Parson-Stream-Format", label))
        .insert_header(("X-Parson-Live", "1"))
        .streaming(
            relayed
                .body
                .map(|chunk| chunk.map_err(actix_web::error::ErrorBadGateway)),
        )
}

/// Streams a station to a renderer (signed zone URLs carry no listener).
pub async fn stream_station_to(
    request: &HttpRequest,
    song_id: &str,
    destination: Destination,
) -> HttpResponse {
    let Some(station_id) = station_id_from_song(song_id) else {
        return not_found("Station not found.", "station_not_found");
    };
    let radio = service(request.app_data::<web::Data<LiveRadio>>().cloned());
    let pool = request
        .app_data::<web::Data<DbPool>>()
        .map(|pool| pool.get_ref().clone());
    let pool = pool.as_ref();
    let Some(station) = radio.find(pool, station_id, None, true).await else {
        return not_found("Station not found.", "station_not_found");
    };
    match radio.open(&station, &destination).await {
        Ok(relayed) => live_response(relayed),
        Err(error) => relay_error(&error),
    }
}

/// The signal path for a station (`/media/songs/radio:<id>/signal-path`).
pub async fn station_signal_path(
    request: &HttpRequest,
    song_id: &str,
    target: StreamTarget,
) -> HttpResponse {
    let user_id = match authenticated_user_id(request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(station_id) = station_id_from_song(song_id) else {
        return not_found("Station not found.", "station_not_found");
    };
    let radio = service(request.app_data::<web::Data<LiveRadio>>().cloned());
    let pool = request
        .app_data::<web::Data<DbPool>>()
        .map(|pool| pool.get_ref().clone());
    let Some(station) = radio
        .find(pool.as_ref(), station_id, Some(user_id), true)
        .await
    else {
        return not_found("Station not found.", "station_not_found");
    };
    let playing = radio.now_playing(station_id);
    HttpResponse::Ok().json(crate::live_radio::signal::describe(
        &station,
        playing.format.as_ref(),
        playing.transport,
        playing.delivery,
        target,
        None,
    ))
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/live-radio")
            .service(home)
            .service(categories)
            .service(search)
            .service(station_detail)
            .service(verify_station)
            .service(facets)
            .service(now_playing)
            .service(check_stream)
            .service(my_stations)
            .service(add_custom_station)
            .service(save_directory_station)
            .service(edit_custom_station)
            .service(remove_station)
            .service(reorder)
            .service(recent_stations)
            .service(clear_recent),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use actix_web::dev::Service;
    use actix_web::http::StatusCode;
    use actix_web::{App, HttpMessage, test, web};
    use serde_json::{Value, json};

    use super::{accepted_codecs, configure};
    use crate::api::auth::Claims;
    use crate::live_radio::format::tests::mp3_frames;
    use crate::live_radio::guard::fakes::{FakeFetch, FakeReply};
    use crate::live_radio::store::tests::pool;

    fn claims(user: i32, role: &str) -> Claims {
        Claims {
            sub: user.to_string(),
            exp: usize::MAX,
            username: format!("user{user}"),
            bitrate: 0,
            token_type: "access".into(),
            role: role.into(),
            token_version: 0,
            session_id: None,
        }
    }

    macro_rules! app {
        ($radio:expr, $pool:expr, $user:expr, $role:expr) => {{
            let user = $user;
            let role: &'static str = $role;
            test::init_service(
                App::new()
                    .app_data(web::Data::new($pool.clone()))
                    .app_data(web::Data::from($radio.clone()))
                    .wrap_fn(move |request, service| {
                        request.extensions_mut().insert(claims(user, role));
                        service.call(request)
                    })
                    // As in the server: the logo route goes before the
                    // `/live-radio` scope, which would answer 404 for it.
                    .route(
                        "/live-radio/stations/{id}/logo",
                        web::get().to(super::station_logo),
                    )
                    .configure(configure)
                    .route(
                        "/media/songs/{song}/stream",
                        web::get().to(
                            |request: actix_web::HttpRequest, song: web::Path<String>| async move {
                                super::stream_station(&request, &song).await
                            },
                        ),
                    ),
            )
            .await
        }};
    }

    fn fetch() -> FakeFetch {
        FakeFetch::default()
            .with(
                "https://radio.example/live",
                FakeReply::ok("audio/mpeg", mp3_frames(256, 10)).header("icy-name", "Example"),
            )
            .with(
                "https://img.example/logo.png",
                FakeReply::ok("image/png", b"\x89PNG\r\n\x1a\nxx".to_vec()),
            )
    }

    #[actix_web::test]
    async fn browsing_searching_and_saving_directory_stations() {
        let radio = crate::live_radio::tests::service(fetch());
        let pool = pool();
        let app = app!(radio, pool, 2, "user");

        let home: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/live-radio/home?country=fr")
                .to_request(),
        )
        .await;
        assert_eq!(home["popular"][0]["name"], "Classic Vinyl HD");
        assert_eq!(home["popular"][0]["quality"]["tier"], "high");
        assert_eq!(home["lossless"][0]["quality"]["tier"], "lossless");
        assert_eq!(home["categories"][0]["id"], "lossless");
        assert!(home["directory_error"].is_null());
        assert!(home["popular"][0].get("favicon").is_none());

        let page: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/live-radio/stations?tag=jazz&order=votes&limit=10")
                .to_request(),
        )
        .await;
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["has_more"], false);

        let id = "d1a54d2e-623e-4970-ab11-35f7b56c5ec3";
        let saved = test::call_service(
            &app,
            test::TestRequest::put()
                .uri(&format!("/live-radio/my/{id}"))
                .to_request(),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK);
        let my: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get().uri("/live-radio/my").to_request(),
        )
        .await;
        assert_eq!(my["items"][0]["id"], id);
        assert_eq!(my["items"][0]["saved"], true);
        assert_eq!(my["items"][0]["song_id"], format!("radio:{id}"));

        let detail: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri(&format!("/live-radio/stations/{id}"))
                .to_request(),
        )
        .await;
        assert_eq!(detail["station"]["saved"], true);
        assert_eq!(detail["now_playing"]["live"], false);

        let tags: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/live-radio/facets/tags")
                .to_request(),
        )
        .await;
        assert_eq!(tags["items"][0]["name"], "pop");
        let missing = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/live-radio/facets/secrets")
                .to_request(),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let removed = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri(&format!("/live-radio/my/{id}"))
                .to_request(),
        )
        .await;
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    }

    #[actix_web::test]
    async fn custom_stations_are_checked_saved_streamed_and_private() {
        let radio = crate::live_radio::tests::service(fetch());
        let pool = pool();
        let app = app!(radio, pool, 2, "user");

        let lan = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/live-radio/my")
                .set_json(json!({"url": "http://192.168.1.4/stream", "allow_lan": true}))
                .to_request(),
        )
        .await;
        assert_eq!(lan.status(), StatusCode::FORBIDDEN);

        let private = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/live-radio/my")
                .set_json(json!({"url": "http://10.0.0.1/stream"}))
                .to_request(),
        )
        .await;
        assert_eq!(private.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: Value = test::read_body_json(private).await;
        assert_eq!(body["code"], "stream_address_blocked");

        let added: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/live-radio/my")
                .set_json(json!({
                    "url": "https://radio.example/live",
                    "logo_url": "https://img.example/logo.png",
                    "tags": ["Jazz", "jazz", " Late Night "]
                }))
                .to_request(),
        )
        .await;
        assert_eq!(added["station"]["name"], "Example");
        assert_eq!(added["station"]["source"], "custom");
        assert_eq!(added["station"]["tags"], json!(["jazz", "late night"]));
        assert_eq!(added["check"]["quality"]["label"], "MP3 256");
        let id = added["station"]["id"].as_str().unwrap().to_string();

        let renamed: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::patch()
                .uri(&format!("/live-radio/my/{id}"))
                .set_json(json!({"name": "My Jazz"}))
                .to_request(),
        )
        .await;
        assert_eq!(renamed["name"], "My Jazz");

        let stream = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/media/songs/radio:{id}/stream"))
                .to_request(),
        )
        .await;
        assert_eq!(stream.status(), StatusCode::OK);
        assert_eq!(stream.headers().get("content-type").unwrap(), "audio/mpeg");
        assert_eq!(
            stream.headers().get("x-parson-stream-format").unwrap(),
            "MP3 256"
        );
        let bytes = test::read_body(stream).await;
        assert_eq!(bytes.len(), mp3_frames(256, 10).len());

        let logo = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/live-radio/stations/{id}/logo"))
                .to_request(),
        )
        .await;
        assert_eq!(logo.status(), StatusCode::OK);
        assert_eq!(logo.headers().get("content-type").unwrap(), "image/png");
        assert_eq!(
            logo.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );

        // The station is played: it shows in recents.
        actix_web::rt::time::sleep(std::time::Duration::from_millis(50)).await;
        let recent: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/live-radio/recent")
                .to_request(),
        )
        .await;
        assert_eq!(recent["items"][0]["id"], id);

        // Another listener cannot see or play it.
        radio.forget(&id);
        let other = app!(radio, pool, 1, "admin");
        let hidden = test::call_service(
            &other,
            test::TestRequest::get()
                .uri(&format!("/media/songs/radio:{id}/stream"))
                .to_request(),
        )
        .await;
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
        let edit = test::call_service(
            &other,
            test::TestRequest::patch()
                .uri(&format!("/live-radio/my/{id}"))
                .set_json(json!({"name": "Mine now"}))
                .to_request(),
        )
        .await;
        assert_eq!(edit.status(), StatusCode::NOT_FOUND);
        let _ = Arc::strong_count(&radio);
    }

    #[actix_web::test]
    async fn accept_lists_map_container_names_to_codecs() {
        assert_eq!(
            accepted_codecs("accept=flac,mp3,ogg&bitrate=0").unwrap(),
            ["flac", "mp3", "vorbis", "opus"]
        );
        assert_eq!(
            accepted_codecs("accept=flac,aac,vorbis").unwrap(),
            ["flac", "aac", "vorbis"]
        );
        assert!(accepted_codecs("bitrate=0").is_none());
    }
}
