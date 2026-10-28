//! A profile's appearance, mounted under `/users`: `GET /users/me/appearance`
//! and `PUT /users/me/appearance`. The web app, the desktop app and the
//! phone app all read and write this one record, so a theme chosen on one
//! device follows the profile to the others. The server normalises every
//! value (unknown presets fall back, numbers are clamped, Home rows are
//! de-duplicated) with the same rules as the SDK's `normalizeAppearance`.
//!
//! Every save is also announced on the zone event socket
//! (`/zones/events`, `{"type":"appearance_changed","revision":N}`), so other
//! open sessions of the same profile restyle at once instead of on reload.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::{HttpRequest, HttpResponse, get, put, web};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::auth::authenticated_user_id;
use crate::api::error::{bad_request, internal_server_error};
use crate::persistence::connection::DbPool;

pub const PRESETS: [&str; 6] = [
    "mujin-dark",
    "mujin-light",
    "material-you",
    "frosted-glass",
    "oled-black",
    "high-contrast",
];
const COLOR_SCHEMES: [&str; 3] = ["system", "dark", "light"];
const DENSITIES: [&str; 2] = ["comfortable", "compact"];
const FONTS: [&str; 6] = [
    "mujin",
    "inter",
    "fraunces",
    "atkinson",
    "plex",
    "roboto-flex",
];
const NOW_PLAYING_LAYOUTS: [&str; 3] = ["art-left", "art-centred", "immersive"];
const SIDEBAR_MODES: [&str; 2] = ["expanded", "compact"];
const MOTION: [&str; 2] = ["system", "reduced"];
const TRANSPARENCY: [&str; 2] = ["system", "reduced"];
const GLASS_MATERIALS: [&str; 2] = ["mica", "acrylic"];
pub const HOME_ROWS: [&str; 10] = [
    "daily-picks",
    "recently-added",
    "recently-played",
    "new-for-you",
    "similar-artists",
    "best-years",
    "recommended-songs",
    "genres",
    "live-radio",
    "albums-you-might-like",
];
const MAX_SETTINGS_BYTES: usize = 16 * 1024;
const APPEARANCE_EVENT_BUFFER: usize = 64;

/// A profile saved a new appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceChange {
    pub user_id: i32,
    pub revision: i64,
}

static APPEARANCE_EVENTS: OnceLock<tokio::sync::broadcast::Sender<AppearanceChange>> =
    OnceLock::new();

fn appearance_events() -> &'static tokio::sync::broadcast::Sender<AppearanceChange> {
    APPEARANCE_EVENTS.get_or_init(|| tokio::sync::broadcast::channel(APPEARANCE_EVENT_BUFFER).0)
}

fn notify_appearance_changed(change: AppearanceChange) {
    let _ = appearance_events().send(change);
}

/// Subscribes to appearance saves (every profile; filter by `user_id`).
pub fn subscribe_appearance_changes() -> tokio::sync::broadcast::Receiver<AppearanceChange> {
    appearance_events().subscribe()
}

/// The zone socket message for a save.
pub fn appearance_event_message(change: AppearanceChange) -> String {
    serde_json::json!({"type": "appearance_changed", "revision": change.revision}).to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HomeRow {
    pub id: String,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub version: u8,
    pub preset: String,
    pub color_scheme: String,
    pub accent: Option<String>,
    pub accent_from_artwork: bool,
    pub corner_radius: u8,
    pub text_scale: f64,
    pub density: String,
    pub font: String,
    pub home_rows: Vec<HomeRow>,
    pub now_playing_layout: String,
    pub sidebar: String,
    pub motion: String,
    pub transparency: String,
    pub glass_material: String,
    pub follow_system_colors: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        normalize(&Value::Null)
    }
}

fn pick(allowed: &[&str], value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|value| allowed.contains(value))
        .unwrap_or(fallback)
        .to_string()
}

/// `#rrggbb` in lower case, from `#rgb` or `#rrggbb`.
pub fn normalize_hex(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim().trim_start_matches('#');
    if !text.chars().all(|character| character.is_ascii_hexdigit()) {
        return None;
    }
    match text.len() {
        3 => Some(format!(
            "#{}",
            text.chars()
                .flat_map(|character| [character, character])
                .collect::<String>()
                .to_ascii_lowercase()
        )),
        6 => Some(format!("#{}", text.to_ascii_lowercase())),
        _ => None,
    }
}

/// Every known Home row once, in the saved order; rows missing from the
/// saved list come back at their default place, unknown ids are dropped.
pub fn normalize_home_rows(value: Option<&Value>) -> Vec<HomeRow> {
    let mut rows: Vec<HomeRow> = Vec::new();
    for entry in value.and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !HOME_ROWS.contains(&id) || rows.iter().any(|row| row.id == id) {
            continue;
        }
        rows.push(HomeRow {
            id: id.to_string(),
            visible: entry.get("visible").and_then(Value::as_bool) != Some(false),
        });
    }
    for (default_index, id) in HOME_ROWS.iter().enumerate() {
        if rows.iter().any(|row| row.id == *id) {
            continue;
        }
        let at = HOME_ROWS[..default_index]
            .iter()
            .rev()
            .find_map(|earlier| rows.iter().position(|row| row.id == *earlier))
            .map(|position| position + 1)
            .unwrap_or(0);
        rows.insert(
            at,
            HomeRow {
                id: (*id).to_string(),
                visible: true,
            },
        );
    }
    rows
}

/// A complete, valid appearance from any JSON value.
pub fn normalize(value: &Value) -> Appearance {
    let field = |name: &str| value.get(name);
    let corner_radius = field("corner_radius")
        .and_then(Value::as_f64)
        .filter(|radius| radius.is_finite())
        .map(|radius| radius.round().clamp(0.0, 24.0) as u8)
        .unwrap_or(8);
    let text_scale = field("text_scale")
        .and_then(Value::as_f64)
        .filter(|scale| scale.is_finite())
        .map(|scale| (scale.clamp(0.85, 1.3) * 100.0).round() / 100.0)
        .unwrap_or(1.0);
    Appearance {
        version: 1,
        preset: pick(&PRESETS, field("preset"), "mujin-dark"),
        color_scheme: pick(&COLOR_SCHEMES, field("color_scheme"), "system"),
        accent: normalize_hex(field("accent")),
        accent_from_artwork: field("accent_from_artwork").and_then(Value::as_bool) == Some(true),
        corner_radius,
        text_scale,
        density: pick(&DENSITIES, field("density"), "comfortable"),
        font: pick(&FONTS, field("font"), "mujin"),
        home_rows: normalize_home_rows(field("home_rows")),
        now_playing_layout: pick(
            &NOW_PLAYING_LAYOUTS,
            field("now_playing_layout"),
            "art-left",
        ),
        sidebar: pick(&SIDEBAR_MODES, field("sidebar"), "expanded"),
        motion: pick(&MOTION, field("motion"), "system"),
        transparency: pick(&TRANSPARENCY, field("transparency"), "system"),
        glass_material: pick(&GLASS_MATERIALS, field("glass_material"), "mica"),
        follow_system_colors: field("follow_system_colors").and_then(Value::as_bool) != Some(false),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AppearanceRecord {
    pub appearance: Appearance,
    /// 0 until the profile saves an appearance; then 1, 2, ...
    pub revision: i64,
    /// Unix seconds of the last save, when there has been one.
    pub updated_at: Option<i64>,
}

#[derive(QueryableByName)]
struct StoredRow {
    #[diesel(sql_type = Text)]
    settings_json: String,
    #[diesel(sql_type = BigInt)]
    revision: i64,
    #[diesel(sql_type = BigInt)]
    updated_at: i64,
}

pub fn load(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<AppearanceRecord> {
    let row = diesel::sql_query(
        "SELECT settings_json, revision, updated_at FROM user_appearance WHERE user_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .load::<StoredRow>(connection)?
    .pop();
    Ok(match row {
        Some(row) => AppearanceRecord {
            appearance: normalize(&serde_json::from_str(&row.settings_json).unwrap_or(Value::Null)),
            revision: row.revision,
            updated_at: Some(row.updated_at),
        },
        None => AppearanceRecord {
            appearance: Appearance::default(),
            revision: 0,
            updated_at: None,
        },
    })
}

#[derive(Debug)]
pub enum SaveError {
    /// Another device saved first; carries what is stored now.
    Conflict(Box<AppearanceRecord>),
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for SaveError {
    fn from(error: diesel::result::Error) -> Self {
        SaveError::Database(error)
    }
}

/// Stores a normalised appearance. With `base_revision`, the save only
/// applies when nobody saved since that revision.
pub fn save(
    connection: &mut SqliteConnection,
    user_id: i32,
    appearance: &Appearance,
    base_revision: Option<i64>,
    now: i64,
) -> Result<AppearanceRecord, SaveError> {
    connection.immediate_transaction(|connection| {
        let current = load(connection, user_id)?;
        if let Some(base) = base_revision
            && base != current.revision
        {
            return Err(SaveError::Conflict(Box::new(current)));
        }
        let json = serde_json::to_string(appearance).unwrap_or_else(|_| "{}".into());
        let revision = current.revision + 1;
        diesel::sql_query(
            "INSERT INTO user_appearance (user_id, settings_json, revision, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(user_id) DO UPDATE SET settings_json = excluded.settings_json,
               revision = excluded.revision, updated_at = excluded.updated_at",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(&json)
        .bind::<BigInt, _>(revision)
        .bind::<BigInt, _>(now)
        .execute(connection)?;
        Ok(AppearanceRecord {
            appearance: appearance.clone(),
            revision,
            updated_at: Some(now),
        })
    })
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn failed(context: &str, error: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(%error, "{context}");
    internal_server_error(
        "Could not load or save the appearance.",
        "appearance_failed",
    )
}

#[get("/me/appearance")]
pub async fn get_appearance(request: HttpRequest, pool: web::Data<DbPool>) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let pool = pool.get_ref().clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        load(&mut connection, user_id).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(record)) => HttpResponse::Ok()
            .insert_header(("cache-control", "no-store"))
            .json(record),
        Ok(Err(error)) => failed("could not load the appearance", error),
        Err(error) => failed("appearance worker failed", error),
    }
}

#[derive(Deserialize)]
pub struct SaveAppearanceRequest {
    appearance: Value,
    #[serde(default)]
    base_revision: Option<i64>,
}

#[put("/me/appearance")]
pub async fn put_appearance(
    request: HttpRequest,
    body: web::Json<SaveAppearanceRequest>,
    pool: web::Data<DbPool>,
) -> HttpResponse {
    let user_id = match authenticated_user_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !body.appearance.is_object() {
        return bad_request("Send the appearance as an object.", "invalid_appearance");
    }
    if body.appearance.to_string().len() > MAX_SETTINGS_BYTES {
        return bad_request("The appearance is too large.", "appearance_too_large");
    }
    let appearance = normalize(&body.appearance);
    let pool = pool.get_ref().clone();
    let base = body.base_revision;
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        Ok::<_, String>(save(
            &mut connection,
            user_id,
            &appearance,
            base,
            now_seconds(),
        ))
    })
    .await
    {
        Ok(Ok(Ok(record))) => {
            notify_appearance_changed(AppearanceChange {
                user_id,
                revision: record.revision,
            });
            HttpResponse::Ok().json(record)
        }
        Ok(Ok(Err(SaveError::Conflict(current)))) => {
            HttpResponse::Conflict().json(serde_json::json!({
                "error": "appearance_conflict",
                "message": "The appearance changed on another device.",
                "current": current,
            }))
        }
        Ok(Ok(Err(SaveError::Database(error)))) => failed("could not save the appearance", error),
        Ok(Err(error)) => failed("could not save the appearance", error),
        Err(error) => failed("appearance worker failed", error),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(get_appearance).service(put_appearance);
}

#[cfg(test)]
mod tests {
    use actix_web::{App, HttpMessage, http::StatusCode, test as actix_test};
    use diesel::connection::SimpleConnection;
    use serde_json::json;

    use super::*;
    use crate::api::auth::Claims;
    use crate::persistence::connection::migrated_memory_pool;

    fn pool() -> DbPool {
        let pool = migrated_memory_pool();
        pool.get()
            .unwrap()
            .batch_execute(
                "INSERT INTO user (id, username, password, role) VALUES
                   (7, 'alex', 'x', 'admin'), (8, 'sam', 'x', 'user');",
            )
            .unwrap();
        pool
    }

    macro_rules! as_user {
        ($request:expr, $user:expr) => {{
            let request = $request.to_request();
            request.extensions_mut().insert(Claims {
                sub: $user.to_string(),
                exp: usize::MAX,
                username: "listener".to_string(),
                bitrate: 0,
                token_type: "access".to_string(),
                role: "user".to_string(),
                token_version: 0,
                session_id: None,
            });
            request
        }};
    }

    #[test]
    fn unknown_values_fall_back_and_numbers_are_clamped() {
        let appearance = normalize(&json!({
            "preset": "winamp-classic",
            "color_scheme": "light",
            "accent": "#ABC",
            "corner_radius": 99,
            "text_scale": 0.2,
            "density": "compact",
            "font": "comic-sans",
            "now_playing_layout": "immersive",
            "home_rows": [
                {"id": "genres", "visible": false},
                {"id": "nope", "visible": true},
                {"id": "genres", "visible": true},
                {"id": "daily-picks"}
            ],
        }));
        assert_eq!(appearance.preset, "mujin-dark");
        assert_eq!(appearance.color_scheme, "light");
        assert_eq!(appearance.accent.as_deref(), Some("#aabbcc"));
        assert_eq!(appearance.corner_radius, 24);
        assert_eq!(appearance.text_scale, 0.85);
        assert_eq!(appearance.density, "compact");
        assert_eq!(appearance.font, "mujin");
        assert_eq!(appearance.now_playing_layout, "immersive");
        // Saved order first, duplicates dropped, the rest at their places.
        let ids = appearance
            .home_rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), HOME_ROWS.len());
        assert_eq!(ids[0], "genres");
        assert!(!appearance.home_rows[0].visible);
        // Rows missing from the saved list follow their default predecessor.
        assert_eq!(ids[1], "live-radio");
        let daily = ids.iter().position(|id| *id == "daily-picks").unwrap();
        assert_eq!(ids[daily + 1], "recently-added");
        assert!(appearance.home_rows[daily].visible);
        assert_eq!(normalize_hex(Some(&json!("red"))), None);
        assert_eq!(Appearance::default().preset, "mujin-dark");
    }

    #[actix_web::test]
    async fn a_save_is_announced_to_the_profiles_other_sessions() {
        let pool = pool();
        pool.get()
            .unwrap()
            .batch_execute(
                "INSERT INTO user (id, username, password, role) VALUES (41, 'kai', 'x', 'user');",
            )
            .unwrap();
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .service(put_appearance),
        )
        .await;
        let mut events = subscribe_appearance_changes();
        let save = |base: i64| {
            as_user!(
                actix_test::TestRequest::put()
                    .uri("/me/appearance")
                    .set_json(
                        json!({"appearance": {"preset": "oled-black"}, "base_revision": base})
                    ),
                41
            )
        };
        assert_eq!(
            actix_test::call_service(&app, save(0)).await.status(),
            StatusCode::OK
        );
        // A save that lost a race changes nothing, so nothing is announced.
        assert_eq!(
            actix_test::call_service(&app, save(0)).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            actix_test::call_service(&app, save(1)).await.status(),
            StatusCode::OK
        );
        // Other tests save too; keep only this profile's announcements.
        let mut mine = Vec::new();
        while let Ok(change) = events.try_recv() {
            if change.user_id == 41 {
                mine.push(change.revision);
            }
        }
        assert_eq!(mine, vec![1, 2]);
        let message: Value = serde_json::from_str(&appearance_event_message(AppearanceChange {
            user_id: 41,
            revision: 2,
        }))
        .unwrap();
        assert_eq!(
            message,
            json!({"type": "appearance_changed", "revision": 2})
        );
    }

    #[actix_web::test]
    async fn each_profile_keeps_its_own_appearance_and_conflicts_are_reported() {
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(pool()))
                .service(get_appearance)
                .service(put_appearance),
        )
        .await;
        let fresh = actix_test::call_service(
            &app,
            as_user!(actix_test::TestRequest::get().uri("/me/appearance"), 7),
        )
        .await;
        assert_eq!(fresh.status(), StatusCode::OK);
        let fresh: Value = actix_test::read_body_json(fresh).await;
        assert_eq!(fresh["revision"], 0);
        assert_eq!(fresh["appearance"]["preset"], "mujin-dark");

        let saved = actix_test::call_service(
            &app,
            as_user!(
                actix_test::TestRequest::put()
                    .uri("/me/appearance")
                    .set_json(json!({
                        "appearance": {"preset": "material-you", "accent": "#3366FF"},
                        "base_revision": 0
                    })),
                7
            ),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK);
        let saved: Value = actix_test::read_body_json(saved).await;
        assert_eq!(saved["revision"], 1);
        assert_eq!(saved["appearance"]["accent"], "#3366ff");

        // Another device still on revision 0 is told what changed.
        let stale = actix_test::call_service(
            &app,
            as_user!(
                actix_test::TestRequest::put()
                    .uri("/me/appearance")
                    .set_json(json!({"appearance": {"preset": "oled-black"}, "base_revision": 0})),
                7
            ),
        )
        .await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let stale: Value = actix_test::read_body_json(stale).await;
        assert_eq!(stale["current"]["appearance"]["preset"], "material-you");

        // The other profile is untouched.
        let other = actix_test::call_service(
            &app,
            as_user!(actix_test::TestRequest::get().uri("/me/appearance"), 8),
        )
        .await;
        let other: Value = actix_test::read_body_json(other).await;
        assert_eq!(other["appearance"]["preset"], "mujin-dark");
        assert_eq!(other["revision"], 0);

        let invalid = actix_test::call_service(
            &app,
            as_user!(
                actix_test::TestRequest::put()
                    .uri("/me/appearance")
                    .set_json(json!({"appearance": "dark"})),
                7
            ),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }
}
