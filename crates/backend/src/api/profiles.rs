//! Profiles: several people on one Parson server.
//!
//! A profile is a Parson account, so each person's history, favourites,
//! ratings, radio taste, recommendations and scrobbling connections stay
//! their own. Besides full accounts (which sign in with a password) anyone
//! in the household can add *managed profiles*: accounts without a usable
//! password that are entered by switching from a device that is already
//! signed in. Switching is per device: it replaces only this device's
//! session, and playback keeps going.
//!
//! Switching asks for nothing, a PIN or the account password:
//! - a profile with a PIN asks for the PIN;
//! - an administrator without a PIN asks for the password;
//! - an account hidden from the switcher asks for the password;
//! - everyone else switches instantly, like Roon.

use actix_web::{HttpMessage, HttpRequest, HttpResponse, delete, get, patch, post, web};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::{Deserialize, Serialize};

use crate::api::auth::{
    Claims, SessionIdentity, authenticated_user_id, clear_auth_attempts, hash_password,
    issue_session, record_auth_attempt, verify_password,
};
use crate::api::error::{bad_request, forbidden, internal_server_error, not_found, unauthorized};
use crate::persistence::connection::DbPool;

/// Profile colours the clients render as avatar gradients.
pub const PROFILE_COLORS: &[&str] = &[
    "violet", "indigo", "sky", "teal", "emerald", "lime", "amber", "orange", "rose", "pink",
    "slate",
];
const MAX_MANAGED_PROFILES: i64 = 32;
const SWITCH_ATTEMPTS_PER_MINUTE: usize = 6;
const MAX_NAME_CHARACTERS: usize = 40;

#[derive(Clone, Debug, QueryableByName)]
pub(crate) struct ProfileRow {
    #[diesel(sql_type = Integer)]
    pub id: i32,
    #[diesel(sql_type = Text)]
    pub username: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub name: Option<String>,
    #[diesel(sql_type = Text)]
    pub role: String,
    #[diesel(sql_type = Text)]
    pub profile_kind: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub profile_color: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub profile_pin: Option<String>,
    #[diesel(sql_type = Integer)]
    pub quick_switch: i32,
    #[diesel(sql_type = Nullable<Integer>)]
    pub profile_owner_id: Option<i32>,
    #[diesel(sql_type = Integer)]
    pub bitrate: i32,
    #[diesel(sql_type = Integer)]
    pub token_version: i32,
    #[diesel(sql_type = BigInt)]
    pub listens: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub services: Option<String>,
}

const PROFILE_SELECT: &str = "SELECT u.id, u.username, u.name, u.role, u.profile_kind,
       u.profile_color, u.profile_pin, u.quick_switch, u.profile_owner_id, u.bitrate,
       u.token_version,
       (SELECT CAST(COUNT(*) AS BIGINT) FROM listen_history_item h WHERE h.user_id = u.id) AS listens,
       (SELECT group_concat(service, ',') FROM scrobble_connection c
          WHERE c.user_id = u.id AND c.enabled = 1) AS services
     FROM user u";

impl ProfileRow {
    fn managed(&self) -> bool {
        self.profile_kind == "profile"
    }

    fn display_name(&self) -> String {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.username)
            .to_string()
    }
}

/// What switching to a profile asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Unlock {
    None,
    Pin,
    Password,
}

pub(crate) fn unlock_requirement(target: &ProfileRow, current_id: i32) -> Unlock {
    if target.id == current_id {
        Unlock::None
    } else if target.profile_pin.is_some() {
        Unlock::Pin
    } else if target.role == "admin" || target.quick_switch == 0 {
        Unlock::Password
    } else {
        Unlock::None
    }
}

/// Whether `requester` may edit `target`'s profile settings.
pub(crate) fn can_edit(requester: &ProfileRow, target: &ProfileRow) -> bool {
    requester.id == target.id
        || requester.role == "admin"
        || (target.managed() && target.profile_owner_id == Some(requester.id))
}

fn can_delete(requester: &ProfileRow, target: &ProfileRow) -> bool {
    requester.id != target.id
        && target.managed()
        && (requester.role == "admin" || target.profile_owner_id == Some(requester.id))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub id: i32,
    pub name: String,
    pub username: String,
    /// "account" signs in with a password; "profile" is managed.
    pub kind: String,
    pub role: String,
    pub color: String,
    pub has_pin: bool,
    pub quick_switch: bool,
    pub is_current: bool,
    pub can_edit: bool,
    pub can_delete: bool,
    pub unlock: Unlock,
    pub listens: i64,
    pub scrobbling: Vec<String>,
}

fn default_color(id: i32) -> &'static str {
    PROFILE_COLORS[(id.unsigned_abs() as usize) % PROFILE_COLORS.len()]
}

fn view(row: &ProfileRow, requester: &ProfileRow) -> ProfileView {
    ProfileView {
        id: row.id,
        name: row.display_name(),
        username: row.username.clone(),
        kind: row.profile_kind.clone(),
        role: row.role.clone(),
        color: row
            .profile_color
            .clone()
            .filter(|color| PROFILE_COLORS.contains(&color.as_str()))
            .unwrap_or_else(|| default_color(row.id).to_string()),
        has_pin: row.profile_pin.is_some(),
        quick_switch: row.quick_switch != 0,
        is_current: row.id == requester.id,
        can_edit: can_edit(requester, row),
        can_delete: can_delete(requester, row),
        unlock: unlock_requirement(row, requester.id),
        listens: row.listens,
        scrobbling: row
            .services
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .filter(|service| !service.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesResponse {
    pub current_id: i32,
    pub can_create: bool,
    pub colors: &'static [&'static str],
    pub profiles: Vec<ProfileView>,
}

pub(crate) fn load_profiles(connection: &mut SqliteConnection) -> QueryResult<Vec<ProfileRow>> {
    diesel::sql_query(format!(
        "{PROFILE_SELECT} ORDER BY u.profile_kind, COALESCE(NULLIF(TRIM(u.name), ''), u.username) COLLATE NOCASE, u.id"
    ))
    .load(connection)
}

pub(crate) fn load_profile(
    connection: &mut SqliteConnection,
    id: i32,
) -> QueryResult<Option<ProfileRow>> {
    diesel::sql_query(format!("{PROFILE_SELECT} WHERE u.id = ?"))
        .bind::<Integer, _>(id)
        .get_result(connection)
        .optional()
}

/// The profiles a requester sees: everyone in the switcher, plus (for
/// administrators) accounts hidden from it.
pub(crate) fn visible_profiles(rows: &[ProfileRow], requester: &ProfileRow) -> Vec<ProfileView> {
    rows.iter()
        .filter(|row| row.quick_switch != 0 || row.id == requester.id || requester.role == "admin")
        .map(|row| view(row, requester))
        .collect()
}

pub(crate) fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= MAX_NAME_CHARACTERS
        && !trimmed.chars().any(char::is_control)
}

pub(crate) fn valid_pin(pin: &str) -> bool {
    (4..=8).contains(&pin.len()) && pin.bytes().all(|byte| byte.is_ascii_digit())
}

/// A unique, readable username for a managed profile ("profile-mia").
pub(crate) fn managed_username(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let slug = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(40)
        .collect::<String>();
    let base = if slug.is_empty() {
        "profile".to_string()
    } else {
        format!("profile-{slug}")
    };
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !taken(candidate))
        .expect("an unused username exists")
}

fn requester_id(request: &HttpRequest) -> Result<i32, HttpResponse> {
    authenticated_user_id(request)
}

async fn db<T: Send + 'static>(
    pool: &web::Data<DbPool>,
    work: impl FnOnce(&mut SqliteConnection) -> QueryResult<T> + Send + 'static,
) -> Result<T, HttpResponse> {
    let pool = pool.get_ref().clone();
    match web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        work(&mut connection).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::error!(%error, "profile storage failed");
            Err(internal_server_error(
                "Profiles are temporarily unavailable.",
                "profiles_unavailable",
            ))
        }
        Err(error) => {
            tracing::error!(%error, "profile worker failed");
            Err(internal_server_error(
                "Profiles are temporarily unavailable.",
                "profiles_unavailable",
            ))
        }
    }
}

#[get("")]
async fn list_profiles(pool: web::Data<DbPool>, request: HttpRequest) -> HttpResponse {
    let requester = match requester_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let rows = match db(&pool, load_profiles).await {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    let Some(current) = rows.iter().find(|row| row.id == requester) else {
        return unauthorized("Your profile no longer exists.", "profile_missing");
    };
    HttpResponse::Ok().json(ProfilesResponse {
        current_id: requester,
        can_create: !current.managed(),
        colors: PROFILE_COLORS,
        profiles: visible_profiles(&rows, current),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProfile {
    name: String,
    color: Option<String>,
    pin: Option<String>,
}

#[post("")]
async fn create_profile(
    body: web::Json<CreateProfile>,
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> HttpResponse {
    let requester = match requester_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let body = body.into_inner();
    if !valid_name(&body.name) {
        return bad_request(
            "Give the profile a name of up to 40 characters.",
            "invalid_profile_name",
        );
    }
    if let Some(color) = body.color.as_deref()
        && !PROFILE_COLORS.contains(&color)
    {
        return bad_request(
            "Choose one of the profile colours.",
            "invalid_profile_color",
        );
    }
    if let Some(pin) = body.pin.as_deref()
        && !valid_pin(pin)
    {
        return bad_request("A PIN is 4 to 8 digits.", "invalid_profile_pin");
    }
    let pin_hash = match body.pin.as_deref().map(hash_password).transpose() {
        Ok(hash) => hash,
        Err(_) => return internal_server_error("Could not save the PIN.", "pin_hash_failed"),
    };
    // Managed profiles never sign in with a password: store the hash of a
    // random value nobody knows.
    let password = match hash_password(&uuid::Uuid::new_v4().to_string()) {
        Ok(hash) => hash,
        Err(_) => {
            return internal_server_error("Could not create the profile.", "profile_create_failed");
        }
    };
    let name = body.name.trim().to_string();
    let color = body.color.clone();
    enum Outcome {
        Created(i32),
        NotAllowed,
        TooMany,
    }
    let outcome = db(&pool, move |connection| {
        connection.immediate_transaction(|connection| {
            let Some(owner) = load_profile(connection, requester)? else {
                return Ok(Outcome::NotAllowed);
            };
            if owner.managed() {
                return Ok(Outcome::NotAllowed);
            }
            #[derive(QueryableByName)]
            struct Count {
                #[diesel(sql_type = BigInt)]
                count: i64,
            }
            let managed = diesel::sql_query(
                "SELECT CAST(COUNT(*) AS BIGINT) AS count FROM user WHERE profile_kind = 'profile'",
            )
            .get_result::<Count>(connection)?
            .count;
            if managed >= MAX_MANAGED_PROFILES {
                return Ok(Outcome::TooMany);
            }
            #[derive(QueryableByName)]
            struct Username {
                #[diesel(sql_type = Text)]
                username: String,
            }
            let taken = diesel::sql_query("SELECT username FROM user")
                .load::<Username>(connection)?
                .into_iter()
                .map(|row| row.username.to_lowercase())
                .collect::<std::collections::HashSet<_>>();
            let username = managed_username(&name, |candidate| taken.contains(candidate));
            #[derive(QueryableByName)]
            struct Id {
                #[diesel(sql_type = Integer)]
                id: i32,
            }
            let id = diesel::sql_query(
                "INSERT INTO user
                   (username, name, password, role, bitrate, profile_kind, profile_color,
                    profile_pin, quick_switch, profile_owner_id)
                 VALUES (?, ?, ?, 'user', ?, 'profile', ?, ?, 1, ?)
                 RETURNING id",
            )
            .bind::<Text, _>(&username)
            .bind::<Text, _>(&name)
            .bind::<Text, _>(&password)
            .bind::<Integer, _>(owner.bitrate)
            .bind::<Nullable<Text>, _>(color.as_deref())
            .bind::<Nullable<Text>, _>(pin_hash.as_deref())
            .bind::<Integer, _>(owner.id)
            .get_result::<Id>(connection)?
            .id;
            Ok(Outcome::Created(id))
        })
    })
    .await;
    match outcome {
        Ok(Outcome::Created(id)) => profile_response(&pool, requester, id, true).await,
        Ok(Outcome::NotAllowed) => forbidden(
            "Switch to a signed-in account to add profiles.",
            "profile_create_forbidden",
        ),
        Ok(Outcome::TooMany) => bad_request(
            "This server already has the most profiles it supports.",
            "too_many_profiles",
        ),
        Err(response) => response,
    }
}

async fn profile_response(
    pool: &web::Data<DbPool>,
    requester: i32,
    id: i32,
    created: bool,
) -> HttpResponse {
    let rows = match db(pool, move |connection| {
        Ok((
            load_profile(connection, requester)?,
            load_profile(connection, id)?,
        ))
    })
    .await
    {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    match rows {
        (Some(requester), Some(target)) => {
            let body = view(&target, &requester);
            if created {
                HttpResponse::Created().json(body)
            } else {
                HttpResponse::Ok().json(body)
            }
        }
        _ => not_found("Profile not found.", "profile_not_found"),
    }
}

/// `pin`: absent leaves the PIN alone, `null` removes it, a string sets it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProfile {
    name: Option<String>,
    color: Option<String>,
    quick_switch: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pin: Option<Option<String>>,
}

fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[patch("/{id}")]
async fn update_profile(
    path: web::Path<i32>,
    body: web::Json<UpdateProfile>,
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> HttpResponse {
    let requester = match requester_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let target_id = path.into_inner();
    let body = body.into_inner();
    if body.name.as_deref().is_some_and(|name| !valid_name(name)) {
        return bad_request(
            "Give the profile a name of up to 40 characters.",
            "invalid_profile_name",
        );
    }
    if body
        .color
        .as_deref()
        .is_some_and(|color| !PROFILE_COLORS.contains(&color))
    {
        return bad_request(
            "Choose one of the profile colours.",
            "invalid_profile_color",
        );
    }
    let pin_hash = match &body.pin {
        Some(Some(pin)) if !valid_pin(pin) => {
            return bad_request("A PIN is 4 to 8 digits.", "invalid_profile_pin");
        }
        Some(Some(pin)) => match hash_password(pin) {
            Ok(hash) => Some(Some(hash)),
            Err(_) => return internal_server_error("Could not save the PIN.", "pin_hash_failed"),
        },
        Some(None) => Some(None),
        None => None,
    };
    let rows = match db(&pool, move |connection| {
        Ok((
            load_profile(connection, requester)?,
            load_profile(connection, target_id)?,
        ))
    })
    .await
    {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    let (Some(requester_row), Some(target)) = rows else {
        return not_found("Profile not found.", "profile_not_found");
    };
    if !can_edit(&requester_row, &target) {
        return forbidden(
            "You can edit your own profile and the profiles you added.",
            "profile_edit_forbidden",
        );
    }
    if body.quick_switch == Some(false) && target.managed() {
        return bad_request(
            "Profiles without a password are always in the switcher.",
            "profile_switcher_required",
        );
    }
    let name = body.name.map(|name| name.trim().to_string());
    let color = body.color;
    let quick_switch = body.quick_switch.map(i32::from);
    if let Err(response) = db(&pool, move |connection| {
        diesel::sql_query(
            "UPDATE user SET
               name = COALESCE(?, name),
               profile_color = COALESCE(?, profile_color),
               quick_switch = COALESCE(?, quick_switch),
               profile_pin = CASE WHEN ? THEN ? ELSE profile_pin END
             WHERE id = ?",
        )
        .bind::<Nullable<Text>, _>(name.as_deref())
        .bind::<Nullable<Text>, _>(color.as_deref())
        .bind::<Nullable<Integer>, _>(quick_switch)
        .bind::<diesel::sql_types::Bool, _>(pin_hash.is_some())
        .bind::<Nullable<Text>, _>(pin_hash.clone().flatten())
        .bind::<Integer, _>(target_id)
        .execute(connection)
    })
    .await
    {
        return response;
    }
    profile_response(&pool, requester, target_id, false).await
}

#[delete("/{id}")]
async fn delete_profile(
    path: web::Path<i32>,
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> HttpResponse {
    let requester = match requester_id(&request) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let target_id = path.into_inner();
    let rows = match db(&pool, move |connection| {
        Ok((
            load_profile(connection, requester)?,
            load_profile(connection, target_id)?,
        ))
    })
    .await
    {
        Ok(rows) => rows,
        Err(response) => return response,
    };
    let (Some(requester_row), Some(target)) = rows else {
        return not_found("Profile not found.", "profile_not_found");
    };
    if !can_delete(&requester_row, &target) {
        return forbidden(
            "Only the person who added a profile, or an administrator, can remove it. Accounts are removed under Users.",
            "profile_delete_forbidden",
        );
    }
    let backup_pool = pool.get_ref().clone();
    match web::block(move || crate::api::data::create_safety_backup(&backup_pool)).await {
        Ok(Ok(())) => {}
        _ => {
            return internal_server_error(
                "Could not create a safety backup, so the profile was not removed.",
                "profile_delete_backup_failed",
            );
        }
    }
    if let Err(response) = db(&pool, move |connection| {
        connection.immediate_transaction(|connection| {
            diesel::sql_query("UPDATE user SET profile_owner_id = NULL WHERE profile_owner_id = ?")
                .bind::<Integer, _>(target_id)
                .execute(connection)?;
            crate::api::user::remove_user_rows(connection, target_id)
        })
    })
    .await
    {
        return response;
    }
    crate::api::auth::invalidate_image_session(target_id).await;
    let avatar = crate::library::storage::profile_picture_file(target_id);
    if let Err(error) = tokio::fs::remove_file(avatar).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, target_id, "could not remove a deleted profile's avatar");
    }
    HttpResponse::NoContent().finish()
}

#[derive(Deserialize, Default)]
struct SwitchProfile {
    pin: Option<String>,
    password: Option<String>,
}

/// Switches this device to another profile: checks the PIN or password the
/// profile asks for, then replaces this device's session.
#[post("/{id}/switch")]
async fn switch_profile(
    path: web::Path<i32>,
    body: Option<web::Json<SwitchProfile>>,
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> HttpResponse {
    let Some(claims) = request.extensions().get::<Claims>().cloned() else {
        return unauthorized("Sign in first.", "session_required");
    };
    let requester = match claims.sub.parse::<i32>() {
        Ok(id) => id,
        Err(_) => return unauthorized("Sign in first.", "session_required"),
    };
    let target_id = path.into_inner();
    let body = body.map(web::Json::into_inner).unwrap_or_default();
    let target = match db(&pool, move |connection| load_profile(connection, target_id)).await {
        Ok(Some(target)) => target,
        Ok(None) => return not_found("Profile not found.", "profile_not_found"),
        Err(response) => return response,
    };
    let unlock = unlock_requirement(&target, requester);
    let attempt_key = if unlock == Unlock::None {
        None
    } else {
        match record_auth_attempt(
            &request,
            &format!("profile-switch:{target_id}"),
            SWITCH_ATTEMPTS_PER_MINUTE,
        ) {
            Ok(key) => Some(key),
            Err(response) => return response,
        }
    };
    let verified = match unlock {
        Unlock::None => true,
        Unlock::Pin | Unlock::Password => {
            let pin = body.pin.clone();
            let password = body.password.clone();
            let pin_hash = target.profile_pin.clone();
            let password_hash = match db(&pool, move |connection| {
                #[derive(QueryableByName)]
                struct Hash {
                    #[diesel(sql_type = Text)]
                    password: String,
                }
                diesel::sql_query("SELECT password FROM user WHERE id = ?")
                    .bind::<Integer, _>(target_id)
                    .get_result::<Hash>(connection)
                    .map(|row| row.password)
            })
            .await
            {
                Ok(hash) => hash,
                Err(response) => return response,
            };
            web::block(move || {
                let pin_ok = matches!((unlock, pin.as_deref(), pin_hash.as_deref()),
                    (Unlock::Pin, Some(pin), Some(hash)) if valid_pin(pin) && verify_password(pin, hash));
                // A password also opens a PIN-protected account.
                pin_ok
                    || password
                        .as_deref()
                        .is_some_and(|password| verify_password(password, &password_hash))
            })
            .await
            .unwrap_or(false)
        }
    };
    if !verified {
        // 403, not 401: a wrong PIN must not look like an expired session
        // (clients refresh and retry on 401).
        return forbidden(
            match unlock {
                Unlock::Pin => "That PIN is not right.",
                _ => "That password is not right.",
            },
            match unlock {
                Unlock::Pin => "profile_pin_incorrect",
                _ => "profile_password_incorrect",
            },
        );
    }
    if let Some(key) = attempt_key {
        clear_auth_attempts(&key);
    }
    let replaces = claims
        .session_id
        .clone()
        .map(|session| (requester, session));
    issue_session(
        &request,
        pool.get_ref().clone(),
        SessionIdentity {
            user_id: target.id,
            username: target.username.clone(),
            bitrate: target.bitrate,
            role: target.role.clone(),
            token_version: target.token_version,
        },
        replaces,
    )
    .await
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/profiles")
            .service(list_profiles)
            .service(create_profile)
            .service(update_profile)
            .service(delete_profile)
            .service(switch_profile),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, http::StatusCode, test as actix_test};
    use diesel::connection::SimpleConnection;
    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel_migrations::MigrationHarness;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn pool() -> DbPool {
        let pool = Arc::new(
            Pool::builder()
                .max_size(1)
                .build(ConnectionManager::<SqliteConnection>::new(":memory:"))
                .expect("pool"),
        );
        let mut connection = pool.get().expect("connection");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        let admin = hash_password("admin-password").unwrap();
        let listener = hash_password("listener-password").unwrap();
        connection
            .batch_execute("PRAGMA foreign_keys = ON;")
            .unwrap();
        diesel::sql_query(
            "INSERT INTO user (id, username, name, password, role) VALUES
               (1, 'dad', 'Dad', ?, 'admin'),
               (2, 'sam', NULL, ?, 'user')",
        )
        .bind::<Text, _>(&admin)
        .bind::<Text, _>(&listener)
        .execute(&mut connection)
        .unwrap();
        drop(connection);
        pool
    }

    fn claims(user_id: i32, session: &str) -> Claims {
        Claims {
            sub: user_id.to_string(),
            exp: usize::MAX,
            username: format!("user-{user_id}"),
            bitrate: 0,
            token_type: "access".into(),
            role: "user".into(),
            token_version: 0,
            session_id: Some(session.into()),
        }
    }

    macro_rules! app {
        ($pool:expr) => {
            actix_test::init_service(
                App::new()
                    .app_data(web::Data::new($pool.clone()))
                    .configure(configure),
            )
            .await
        };
    }

    macro_rules! request {
        ($method:expr, $uri:expr, $user:expr, $body:expr $(,)?) => {{
            let user_id: i32 = $user;
            let body: Option<Value> = $body;
            let mut builder = $method
                .uri(&$uri)
                .peer_addr(format!("10.0.0.{user_id}:5000").parse().unwrap());
            if let Some(body) = body {
                builder = builder.set_json(body);
            }
            let request = builder.to_request();
            request
                .extensions_mut()
                .insert(claims(user_id, "device-session"));
            request
        }};
    }

    #[actix_web::test]
    async fn a_household_creates_lists_and_switches_profiles() {
        let pool = pool();
        let app = app!(pool);

        let created = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles",
                2,
                Some(json!({"name": "Mia ", "color": "rose"})),
            ),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let mia: Value = actix_test::read_body_json(created).await;
        assert_eq!(mia["name"], "Mia");
        assert_eq!(mia["username"], "profile-mia");
        assert_eq!(mia["kind"], "profile");
        assert_eq!(mia["color"], "rose");
        assert_eq!(mia["unlock"], "none");
        assert_eq!(mia["canDelete"], true);
        let mia_id = mia["id"].as_i64().unwrap() as i32;

        let listed: Value = actix_test::call_and_read_body_json(
            &app,
            request!(actix_test::TestRequest::get(), "/profiles", 2, None),
        )
        .await;
        assert_eq!(listed["currentId"], 2);
        let names = listed["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Dad", "sam", "Mia"]);
        let dad = &listed["profiles"][0];
        // An administrator without a PIN asks for the password.
        assert_eq!(dad["unlock"], "password");

        // Sam switches this device to Mia instantly; the old device
        // session is replaced.
        let mut connection = pool.get().unwrap();
        diesel::sql_query(
            "INSERT INTO refresh_session (id, user_id, token_hash, expires_at)
             VALUES ('device-session', 2, 'hash', 9999999999)",
        )
        .execute(&mut connection)
        .unwrap();
        drop(connection);
        let switched = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                &format!("/profiles/{mia_id}/switch"),
                2,
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(switched.status(), StatusCode::OK);
        assert!(
            switched
                .response()
                .cookies()
                .any(|cookie| cookie.name() == "plm_accessToken")
        );
        let body: Value = actix_test::read_body_json(switched).await;
        assert_eq!(body["claims"]["sub"], mia_id.to_string());
        assert_eq!(body["claims"]["username"], "profile-mia");
        #[derive(QueryableByName)]
        struct Sessions {
            #[diesel(sql_type = Integer)]
            user_id: i32,
        }
        let mut connection = pool.get().unwrap();
        let sessions = diesel::sql_query("SELECT user_id FROM refresh_session")
            .load::<Sessions>(&mut connection)
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].user_id, mia_id);
    }

    #[actix_web::test]
    async fn pins_and_passwords_guard_switching() {
        let pool = pool();
        let app = app!(pool);
        // Dad (admin) asks for his password until he sets a PIN.
        let wrong = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles/1/switch",
                2,
                Some(json!({"password": "nope-nope"})),
            ),
        )
        .await;
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        let right = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles/1/switch",
                2,
                Some(json!({"password": "admin-password"})),
            ),
        )
        .await;
        assert_eq!(right.status(), StatusCode::OK);

        let updated: Value = actix_test::call_and_read_body_json(
            &app,
            request!(
                actix_test::TestRequest::patch(),
                "/profiles/1",
                1,
                Some(json!({"pin": "2468"})),
            ),
        )
        .await;
        assert_eq!(updated["hasPin"], true);
        let listed: Value = actix_test::call_and_read_body_json(
            &app,
            request!(actix_test::TestRequest::get(), "/profiles", 2, None),
        )
        .await;
        assert_eq!(listed["profiles"][0]["unlock"], "pin");
        assert_eq!(listed["profiles"][0]["hasPin"], true);

        let bad_pin = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles/1/switch",
                2,
                Some(json!({"pin": "1357"})),
            ),
        )
        .await;
        assert_eq!(bad_pin.status(), StatusCode::FORBIDDEN);
        let body: Value = actix_test::read_body_json(bad_pin).await;
        assert_eq!(body["code"], "profile_pin_incorrect");
        let good_pin = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles/1/switch",
                2,
                Some(json!({"pin": "2468"})),
            ),
        )
        .await;
        assert_eq!(good_pin.status(), StatusCode::OK);

        // Guessing is rate limited per profile.
        let mut last = StatusCode::OK;
        for _ in 0..8 {
            last = actix_test::call_service(
                &app,
                request!(
                    actix_test::TestRequest::post(),
                    "/profiles/1/switch",
                    2,
                    Some(json!({"pin": "0000"})),
                ),
            )
            .await
            .status();
        }
        assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);

        // Removing the PIN with null.
        let cleared: Value = actix_test::call_and_read_body_json(
            &app,
            request!(
                actix_test::TestRequest::patch(),
                "/profiles/1",
                1,
                Some(json!({"pin": null})),
            ),
        )
        .await;
        assert_eq!(cleared["hasPin"], false);
    }

    #[actix_web::test]
    async fn only_owners_and_admins_manage_profiles() {
        let pool = pool();
        let app = app!(pool);
        let created: Value = actix_test::call_and_read_body_json(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles",
                1,
                Some(json!({"name": "Guest", "pin": "1234"})),
            ),
        )
        .await;
        let guest = created["id"].as_i64().unwrap();
        assert_eq!(created["unlock"], "pin");

        // Sam can neither edit nor delete Dad's or Guest's profile.
        for uri in ["/profiles/1".to_string(), format!("/profiles/{guest}")] {
            let response = actix_test::call_service(
                &app,
                request!(
                    actix_test::TestRequest::patch(),
                    &uri,
                    2,
                    Some(json!({"name": "Hacked"})),
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let response = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::delete(),
                &format!("/profiles/{guest}"),
                2,
                None,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        // Accounts are not removed here.
        let response = actix_test::call_service(
            &app,
            request!(actix_test::TestRequest::delete(), "/profiles/2", 1, None),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        // Managed profiles stay in the switcher.
        let response = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::patch(),
                &format!("/profiles/{guest}"),
                1,
                Some(json!({"quickSwitch": false})),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // Validation.
        for body in [
            json!({"name": ""}),
            json!({"color": "chartreuse"}),
            json!({"pin": "12"}),
            json!({"pin": "12ab"}),
        ] {
            let response = actix_test::call_service(
                &app,
                request!(
                    actix_test::TestRequest::patch(),
                    "/profiles/2",
                    2,
                    Some(body)
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[actix_web::test]
    async fn a_managed_profile_cannot_add_profiles_and_hidden_accounts_need_a_password() {
        let pool = pool();
        let app = app!(pool);
        let created: Value = actix_test::call_and_read_body_json(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles",
                2,
                Some(json!({"name": "Kid"})),
            ),
        )
        .await;
        let kid = created["id"].as_i64().unwrap() as i32;
        let response = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles",
                kid,
                Some(json!({"name": "Another"})),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // Sam hides from the switcher: the kid no longer sees Sam, and
        // switching to Sam takes Sam's password.
        let hidden: Value = actix_test::call_and_read_body_json(
            &app,
            request!(
                actix_test::TestRequest::patch(),
                "/profiles/2",
                2,
                Some(json!({"quickSwitch": false})),
            ),
        )
        .await;
        assert_eq!(hidden["quickSwitch"], false);
        let listed: Value = actix_test::call_and_read_body_json(
            &app,
            request!(actix_test::TestRequest::get(), "/profiles", kid, None),
        )
        .await;
        assert!(
            listed["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .all(|profile| profile["id"] != 2)
        );
        let response = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles/2/switch",
                kid,
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = actix_test::call_service(
            &app,
            request!(
                actix_test::TestRequest::post(),
                "/profiles/2/switch",
                kid,
                Some(json!({"password": "listener-password"})),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn managed_usernames_are_readable_and_unique() {
        let taken = ["profile-mia", "profile-mia-2"];
        assert_eq!(
            managed_username("Mia", |name| taken.contains(&name)),
            "profile-mia-3"
        );
        assert_eq!(
            managed_username("  Zoë & Léo ", |_| false),
            "profile-zoë-léo"
        );
        assert_eq!(managed_username("!!!", |_| false), "profile");
        assert!(valid_pin("0000") && valid_pin("12345678"));
        assert!(!valid_pin("123") && !valid_pin("123456789") && !valid_pin("12 34"));
        assert!(valid_name("Mia") && !valid_name("   ") && !valid_name(&"x".repeat(41)));
    }
}
