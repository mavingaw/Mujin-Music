//! Persistence for streaming accounts and streaming library items.

use std::collections::HashSet;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Double, Integer, Nullable, Text};
use diesel::sqlite::SqliteConnection;
use serde::Serialize;

use super::{
    ItemKind, ProviderAlbum, ProviderId, ProviderPlaylist, ProviderTrack, StreamQuality, unix_now,
};
use crate::library::search::normalize;

pub const MAX_LIBRARY_PAGE: i64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Configured,
    Pending,
    SignedIn,
    Expired,
    Error,
}

impl AccountStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Pending => "pending",
            Self::SignedIn => "signed_in",
            Self::Expired => "expired",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "signed_in" => Self::SignedIn,
            "expired" => Self::Expired,
            "error" => Self::Error,
            _ => Self::Configured,
        }
    }
}

#[derive(Debug, QueryableByName)]
pub struct AccountRow {
    #[diesel(sql_type = Text)]
    pub provider: String,
    #[diesel(sql_type = Binary)]
    pub sealed_credentials: Vec<u8>,
    #[diesel(sql_type = Nullable<Binary>)]
    pub sealed_session: Option<Vec<u8>>,
    #[diesel(sql_type = Nullable<Binary>)]
    pub sealed_pending: Option<Vec<u8>>,
    #[diesel(sql_type = Nullable<Text>)]
    pub account_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub plan: Option<String>,
    #[diesel(sql_type = Text)]
    pub quality: String,
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub status_message: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub last_library_sync: Option<i64>,
}

impl AccountRow {
    pub fn provider_id(&self) -> Option<ProviderId> {
        ProviderId::parse(&self.provider)
    }

    pub fn status(&self) -> AccountStatus {
        AccountStatus::parse(&self.status)
    }

    pub fn quality(&self) -> StreamQuality {
        StreamQuality::parse(&self.quality).unwrap_or(StreamQuality::Lossless)
    }
}

const ACCOUNT_COLUMNS: &str = "provider, sealed_credentials, sealed_session, sealed_pending, \
     account_name, plan, quality, status, status_message, last_library_sync";

pub fn accounts(connection: &mut SqliteConnection, user_id: i32) -> QueryResult<Vec<AccountRow>> {
    diesel::sql_query(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM provider_account WHERE user_id = ? ORDER BY provider"
    ))
    .bind::<Integer, _>(user_id)
    .load(connection)
}

pub fn account(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
) -> QueryResult<Option<AccountRow>> {
    diesel::sql_query(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM provider_account WHERE user_id = ? AND provider = ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .get_result(connection)
    .optional()
}

/// Stores new credentials. Changing credentials always signs the account
/// out, because tokens belong to the client that issued them.
pub fn save_credentials(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    sealed: &[u8],
    default_quality: StreamQuality,
) -> QueryResult<usize> {
    let now = unix_now();
    diesel::sql_query(
        "INSERT INTO provider_account
           (user_id, provider, sealed_credentials, quality, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'configured', ?, ?)
         ON CONFLICT(user_id, provider) DO UPDATE SET
           sealed_credentials = excluded.sealed_credentials,
           sealed_session = NULL,
           sealed_pending = NULL,
           account_name = NULL,
           plan = NULL,
           status = 'configured',
           status_message = NULL,
           updated_at = excluded.updated_at",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .bind::<Binary, _>(sealed)
    .bind::<Text, _>(default_quality.as_str())
    .bind::<BigInt, _>(now)
    .bind::<BigInt, _>(now)
    .execute(connection)
}

pub fn save_pending(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    sealed: &[u8],
) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE provider_account
         SET sealed_pending = ?, status = CASE WHEN sealed_session IS NULL THEN 'pending' ELSE status END,
             status_message = NULL, updated_at = ?
         WHERE user_id = ? AND provider = ?",
    )
    .bind::<Binary, _>(sealed)
    .bind::<BigInt, _>(unix_now())
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .execute(connection)
}

pub fn save_session(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    sealed: &[u8],
    account_name: &str,
    plan: Option<&str>,
) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE provider_account
         SET sealed_session = ?, sealed_pending = NULL, account_name = ?, plan = ?,
             status = 'signed_in', status_message = NULL, updated_at = ?
         WHERE user_id = ? AND provider = ?",
    )
    .bind::<Binary, _>(sealed)
    .bind::<Text, _>(account_name)
    .bind::<Nullable<Text>, _>(plan)
    .bind::<BigInt, _>(unix_now())
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .execute(connection)
}

pub fn set_status(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    status: AccountStatus,
    message: Option<&str>,
) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE provider_account
         SET status = ?, status_message = ?, updated_at = ?,
             sealed_pending = CASE WHEN ? = 'pending' THEN sealed_pending ELSE NULL END
         WHERE user_id = ? AND provider = ?",
    )
    .bind::<Text, _>(status.as_str())
    .bind::<Nullable<Text>, _>(message)
    .bind::<BigInt, _>(unix_now())
    .bind::<Text, _>(status.as_str())
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .execute(connection)
}

pub fn set_quality(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    quality: StreamQuality,
) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE provider_account SET quality = ?, updated_at = ? WHERE user_id = ? AND provider = ?",
    )
    .bind::<Text, _>(quality.as_str())
    .bind::<BigInt, _>(unix_now())
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .execute(connection)
}

/// Signs out but keeps credentials, quality and library items, so signing in
/// again restores everything.
pub fn sign_out(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE provider_account
         SET sealed_session = NULL, sealed_pending = NULL, account_name = NULL, plan = NULL,
             status = 'configured', status_message = NULL, updated_at = ?
         WHERE user_id = ? AND provider = ?",
    )
    .bind::<BigInt, _>(unix_now())
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .execute(connection)
}

/// Removes the account, its credentials and every library item from it.
pub fn remove_account(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
) -> QueryResult<usize> {
    connection.immediate_transaction(|connection| {
        let items = diesel::sql_query(
            "DELETE FROM provider_library_item WHERE user_id = ? AND provider = ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(provider.as_str())
        .execute(connection)?;
        let copies = diesel::sql_query(
            "DELETE FROM provider_playlist_copy WHERE user_id = ? AND provider = ?",
        )
        .bind::<Integer, _>(user_id)
        .bind::<Text, _>(provider.as_str())
        .execute(connection)?;
        let accounts =
            diesel::sql_query("DELETE FROM provider_account WHERE user_id = ? AND provider = ?")
                .bind::<Integer, _>(user_id)
                .bind::<Text, _>(provider.as_str())
                .execute(connection)?;
        Ok(items + copies + accounts)
    })
}

pub fn set_last_sync(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    at: i64,
) -> QueryResult<usize> {
    diesel::sql_query(
        "UPDATE provider_account SET last_library_sync = ? WHERE user_id = ? AND provider = ?",
    )
    .bind::<BigInt, _>(at)
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .execute(connection)
}

#[derive(Debug, QueryableByName)]
pub struct ServerCredentialRow {
    #[diesel(sql_type = Text)]
    pub provider: String,
    #[diesel(sql_type = Binary)]
    pub sealed_credentials: Vec<u8>,
    #[diesel(sql_type = BigInt)]
    pub updated_at: i64,
}

/// Server-wide credentials the server owner saved from Settings.
pub fn server_credentials(
    connection: &mut SqliteConnection,
) -> QueryResult<Vec<ServerCredentialRow>> {
    diesel::sql_query(
        "SELECT provider, sealed_credentials, updated_at FROM provider_server_credential
         ORDER BY provider",
    )
    .load(connection)
}

pub fn save_server_credentials(
    connection: &mut SqliteConnection,
    provider: ProviderId,
    sealed: &[u8],
    updated_by: i32,
) -> QueryResult<i64> {
    let now = unix_now();
    diesel::sql_query(
        "INSERT INTO provider_server_credential (provider, sealed_credentials, updated_by, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(provider) DO UPDATE SET
           sealed_credentials = excluded.sealed_credentials,
           updated_by = excluded.updated_by,
           updated_at = excluded.updated_at",
    )
    .bind::<Text, _>(provider.as_str())
    .bind::<Binary, _>(sealed)
    .bind::<Integer, _>(updated_by)
    .bind::<BigInt, _>(now)
    .execute(connection)?;
    Ok(now)
}

pub fn remove_server_credentials(
    connection: &mut SqliteConnection,
    provider: ProviderId,
) -> QueryResult<usize> {
    diesel::sql_query("DELETE FROM provider_server_credential WHERE provider = ?")
        .bind::<Text, _>(provider.as_str())
        .execute(connection)
}

#[derive(Debug, QueryableByName)]
struct SourcePreferenceRow {
    #[diesel(sql_type = Text)]
    source: String,
}

/// The listener's source preference, `None` when never set.
pub fn source_preference(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<Option<String>> {
    let rows: Vec<SourcePreferenceRow> =
        diesel::sql_query("SELECT source FROM provider_source_preference WHERE user_id = ?")
            .bind::<Integer, _>(user_id)
            .load(connection)?;
    Ok(rows.into_iter().next().map(|row| row.source))
}

pub fn set_source_preference(
    connection: &mut SqliteConnection,
    user_id: i32,
    source: &str,
) -> QueryResult<usize> {
    diesel::sql_query(
        "INSERT INTO provider_source_preference (user_id, source, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
           source = excluded.source,
           updated_at = excluded.updated_at",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(source)
    .bind::<BigInt, _>(unix_now())
    .execute(connection)
}

/// A saved, read-only copy of a service playlist.
#[derive(Clone, Debug, QueryableByName)]
pub struct PlaylistCopyRow {
    #[diesel(sql_type = Text)]
    pub provider: String,
    #[diesel(sql_type = Text)]
    pub provider_playlist_id: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub owner: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub cover_url: Option<String>,
    #[diesel(sql_type = Integer)]
    pub track_count: i32,
    #[diesel(sql_type = Double)]
    pub duration: f64,
    #[diesel(sql_type = Text)]
    pub payload: String,
    #[diesel(sql_type = BigInt)]
    pub saved_at: i64,
}

pub fn save_playlist_copy(
    connection: &mut SqliteConnection,
    user_id: i32,
    playlist: &ProviderPlaylist,
    saved_at: i64,
) -> QueryResult<usize> {
    let raw = super::parse_entity_id(&playlist.id)
        .map(|(_, raw)| raw)
        .unwrap_or_default();
    let payload = serde_json::to_string(playlist).unwrap_or_else(|_| "{}".into());
    diesel::sql_query(
        "INSERT INTO provider_playlist_copy
           (user_id, provider, provider_playlist_id, title, owner, cover_url, track_count,
            duration, payload, saved_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, provider, provider_playlist_id) DO UPDATE SET
           title = excluded.title, owner = excluded.owner, cover_url = excluded.cover_url,
           track_count = excluded.track_count, duration = excluded.duration,
           payload = excluded.payload, saved_at = excluded.saved_at",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(playlist.provider.as_str())
    .bind::<Text, _>(raw)
    .bind::<Text, _>(&playlist.title)
    .bind::<Nullable<Text>, _>(playlist.owner.as_deref())
    .bind::<Nullable<Text>, _>(playlist.cover_url.as_deref())
    .bind::<Integer, _>(playlist.tracks.len().min(i32::MAX as usize) as i32)
    .bind::<Double, _>(
        playlist
            .tracks
            .iter()
            .map(|track| track.duration)
            .sum::<f64>(),
    )
    .bind::<Text, _>(payload)
    .bind::<BigInt, _>(saved_at)
    .execute(connection)
}

const COPY_COLUMNS: &str = "provider, provider_playlist_id, title, owner, cover_url, \
     track_count, duration, payload, saved_at";

pub fn playlist_copies(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<Vec<PlaylistCopyRow>> {
    diesel::sql_query(format!(
        "SELECT {COPY_COLUMNS} FROM provider_playlist_copy WHERE user_id = ?
         ORDER BY saved_at DESC, title"
    ))
    .bind::<Integer, _>(user_id)
    .load(connection)
}

pub fn playlist_copy(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    raw_id: &str,
) -> QueryResult<Option<PlaylistCopyRow>> {
    diesel::sql_query(format!(
        "SELECT {COPY_COLUMNS} FROM provider_playlist_copy
         WHERE user_id = ? AND provider = ? AND provider_playlist_id = ?"
    ))
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .bind::<Text, _>(raw_id)
    .get_result(connection)
    .optional()
}

pub fn remove_playlist_copy(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    raw_id: &str,
) -> QueryResult<usize> {
    diesel::sql_query(
        "DELETE FROM provider_playlist_copy
         WHERE user_id = ? AND provider = ? AND provider_playlist_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .bind::<Text, _>(raw_id)
    .execute(connection)
}

/// A streaming album or track in a listener's library.
#[derive(Clone, Debug, PartialEq, Serialize, QueryableByName)]
pub struct LibraryItemRow {
    #[diesel(sql_type = Text)]
    pub provider: String,
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = Text)]
    pub provider_item_id: String,
    #[diesel(sql_type = Text)]
    pub title: String,
    #[diesel(sql_type = Text)]
    pub artist_name: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub album_title: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub cover_url: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub release_date: Option<String>,
    #[diesel(sql_type = Double)]
    pub duration: f64,
    #[diesel(sql_type = Integer)]
    pub track_count: i32,
    #[diesel(sql_type = Nullable<Text>)]
    pub format_label: Option<String>,
    #[diesel(sql_type = Text)]
    pub payload: String,
    #[diesel(sql_type = Text)]
    pub origin: String,
    #[diesel(sql_type = BigInt)]
    pub added_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemOrigin {
    /// Added from Parson ("Add to library").
    Parson,
    /// Imported from the service's favorites by a library sync.
    Sync,
}

impl ItemOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::Parson => "parson",
            Self::Sync => "sync",
        }
    }
}

pub struct NewLibraryItem {
    pub provider: ProviderId,
    pub kind: ItemKind,
    pub provider_item_id: String,
    pub title: String,
    pub artist_name: String,
    pub album_title: Option<String>,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub duration: f64,
    pub track_count: i32,
    pub format_label: Option<String>,
    pub payload: String,
}

fn raw_id(id: &str) -> String {
    super::parse_entity_id(id)
        .map(|(_, raw)| raw)
        .unwrap_or_else(|| id.to_string())
}

impl NewLibraryItem {
    pub fn album(album: &ProviderAlbum) -> Self {
        let mut snapshot = album.clone();
        snapshot.tracks.clear();
        snapshot.in_library = true;
        Self {
            provider: album.provider,
            kind: ItemKind::Album,
            provider_item_id: raw_id(&album.id),
            title: album.title.clone(),
            artist_name: album.artist.name.clone(),
            album_title: None,
            cover_url: album.cover_url.clone(),
            release_date: album.release_date.clone(),
            duration: album.duration,
            track_count: album.track_count as i32,
            format_label: album.format.as_ref().map(|format| format.label.clone()),
            payload: serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into()),
        }
    }

    pub fn track(track: &ProviderTrack) -> Self {
        let mut snapshot = track.clone();
        snapshot.in_library = true;
        Self {
            provider: track.provider,
            kind: ItemKind::Track,
            provider_item_id: raw_id(&track.id),
            title: track.title.clone(),
            artist_name: track.artist.name.clone(),
            album_title: track.album.as_ref().map(|album| album.title.clone()),
            cover_url: track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.clone()),
            release_date: None,
            duration: track.duration,
            track_count: 1,
            format_label: track.format.as_ref().map(|format| format.label.clone()),
            payload: serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into()),
        }
    }
}

pub fn upsert_item(
    connection: &mut SqliteConnection,
    user_id: i32,
    item: &NewLibraryItem,
    origin: ItemOrigin,
    added_at: i64,
) -> QueryResult<usize> {
    diesel::sql_query(
        "INSERT INTO provider_library_item
           (user_id, provider, kind, provider_item_id, title, normalized_title, artist_name,
            normalized_artist, album_title, cover_url, release_date, duration, track_count,
            format_label, payload, origin, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, provider, kind, provider_item_id) DO UPDATE SET
           title = excluded.title,
           normalized_title = excluded.normalized_title,
           artist_name = excluded.artist_name,
           normalized_artist = excluded.normalized_artist,
           album_title = excluded.album_title,
           cover_url = excluded.cover_url,
           release_date = excluded.release_date,
           duration = excluded.duration,
           track_count = excluded.track_count,
           format_label = excluded.format_label,
           payload = excluded.payload",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(item.provider.as_str())
    .bind::<Text, _>(item.kind.as_str())
    .bind::<Text, _>(&item.provider_item_id)
    .bind::<Text, _>(&item.title)
    .bind::<Text, _>(normalize(&item.title))
    .bind::<Text, _>(&item.artist_name)
    .bind::<Text, _>(normalize(&item.artist_name))
    .bind::<Nullable<Text>, _>(item.album_title.as_deref())
    .bind::<Nullable<Text>, _>(item.cover_url.as_deref())
    .bind::<Nullable<Text>, _>(item.release_date.as_deref())
    .bind::<Double, _>(item.duration)
    .bind::<Integer, _>(item.track_count)
    .bind::<Nullable<Text>, _>(item.format_label.as_deref())
    .bind::<Text, _>(&item.payload)
    .bind::<Text, _>(origin.as_str())
    .bind::<BigInt, _>(added_at)
    .execute(connection)
}

pub fn remove_item(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    kind: ItemKind,
    provider_item_id: &str,
) -> QueryResult<usize> {
    diesel::sql_query(
        "DELETE FROM provider_library_item
         WHERE user_id = ? AND provider = ? AND kind = ? AND provider_item_id = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .bind::<Text, _>(kind.as_str())
    .bind::<Text, _>(provider_item_id)
    .execute(connection)
}

/// Replaces synced favorites: every listed item is kept or added, and items
/// no longer favorited on the service are removed when `prune` is set (only
/// after a complete fetch).
pub fn apply_sync(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
    kind: ItemKind,
    items: &[NewLibraryItem],
    prune: bool,
) -> QueryResult<(usize, usize)> {
    connection.immediate_transaction(|connection| {
        let now = unix_now();
        let existing = item_ids(connection, user_id, Some(provider), kind)?;
        let mut added = 0;
        for (index, item) in items.iter().enumerate() {
            if !existing.contains(&item.provider_item_id) {
                added += 1;
            }
            // Favorites arrive newest first; keep that order for "recently added".
            upsert_item(
                connection,
                user_id,
                item,
                ItemOrigin::Sync,
                now - index as i64,
            )?;
        }
        let mut removed = 0;
        if prune {
            let keep = items
                .iter()
                .map(|item| item.provider_item_id.as_str())
                .collect::<HashSet<_>>();
            for stale in existing.iter().filter(|id| !keep.contains(id.as_str())) {
                removed += remove_item(connection, user_id, provider, kind, stale)?;
            }
        }
        Ok((added, removed))
    })
}

pub fn item_ids(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: Option<ProviderId>,
    kind: ItemKind,
) -> QueryResult<HashSet<String>> {
    #[derive(QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = Text)]
        provider: String,
        #[diesel(sql_type = Text)]
        provider_item_id: String,
    }
    let rows: Vec<IdRow> = diesel::sql_query(
        "SELECT provider, provider_item_id FROM provider_library_item
         WHERE user_id = ? AND kind = ? AND (? IS NULL OR provider = ?)",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(kind.as_str())
    .bind::<Nullable<Text>, _>(provider.map(ProviderId::as_str))
    .bind::<Nullable<Text>, _>(provider.map(ProviderId::as_str))
    .load(connection)?;
    Ok(rows
        .into_iter()
        .map(|row| match provider {
            Some(_) => row.provider_item_id,
            None => format!("{}:{}", row.provider, row.provider_item_id),
        })
        .collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibrarySort {
    Title,
    Recent,
    Artist,
}

pub struct LibraryQuery<'a> {
    pub provider: Option<ProviderId>,
    pub kind: ItemKind,
    pub text: Option<&'a str>,
    pub sort: LibrarySort,
    pub offset: i64,
    pub limit: i64,
}

fn like_pattern(token: &str) -> String {
    let escaped = token
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

pub fn list_items(
    connection: &mut SqliteConnection,
    user_id: i32,
    query: &LibraryQuery<'_>,
) -> QueryResult<(Vec<LibraryItemRow>, i64)> {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    let tokens = query
        .text
        .map(normalize)
        .unwrap_or_default()
        .split_whitespace()
        .take(8)
        .map(like_pattern)
        .collect::<Vec<_>>();
    let mut filter = String::from("user_id = ? AND kind = ? AND (? IS NULL OR provider = ?)");
    for _ in &tokens {
        filter.push_str(
            " AND (normalized_title || ' ' || normalized_artist || ' ' || \
             lower(coalesce(album_title, ''))) LIKE ? ESCAPE '\\'",
        );
    }
    let order = match query.sort {
        LibrarySort::Title => "normalized_title, normalized_artist, provider",
        LibrarySort::Recent => "added_at DESC, normalized_title",
        LibrarySort::Artist => "normalized_artist, release_date, normalized_title",
    };
    let provider = query.provider.map(ProviderId::as_str);

    let mut count = diesel::sql_query(format!(
        "SELECT COUNT(*) AS count FROM provider_library_item WHERE {filter}"
    ))
    .into_boxed()
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(query.kind.as_str())
    .bind::<Nullable<Text>, _>(provider)
    .bind::<Nullable<Text>, _>(provider);
    for token in &tokens {
        count = count.bind::<Text, _>(token.clone());
    }
    let total = count.get_result::<CountRow>(connection)?.count;

    let mut select = diesel::sql_query(format!(
        "SELECT provider, kind, provider_item_id, title, artist_name, album_title, cover_url,
                release_date, duration, track_count, format_label, payload, origin, added_at
         FROM provider_library_item WHERE {filter} ORDER BY {order} LIMIT ? OFFSET ?"
    ))
    .into_boxed()
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(query.kind.as_str())
    .bind::<Nullable<Text>, _>(provider)
    .bind::<Nullable<Text>, _>(provider);
    for token in &tokens {
        select = select.bind::<Text, _>(token.clone());
    }
    let rows = select
        .bind::<BigInt, _>(query.limit.clamp(1, MAX_LIBRARY_PAGE))
        .bind::<BigInt, _>(query.offset.max(0))
        .load(connection)?;
    Ok((rows, total))
}

/// Streaming albums a listener's library can hold in the unified catalog;
/// TIDAL itself stops at about 10,000 favorites.
pub const MAX_CATALOG_ALBUMS: i64 = 10_000;

/// Every streaming album in a listener's library (up to
/// [`MAX_CATALOG_ALBUMS`]), for the unified album catalog.
pub fn all_albums(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> QueryResult<Vec<LibraryItemRow>> {
    diesel::sql_query(
        "SELECT provider, kind, provider_item_id, title, artist_name, album_title, cover_url,
                release_date, duration, track_count, format_label, payload, origin, added_at
         FROM provider_library_item WHERE user_id = ? AND kind = 'album'
         ORDER BY added_at DESC LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_CATALOG_ALBUMS)
    .load(connection)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LibraryCounts {
    pub albums: i64,
    pub tracks: i64,
}

pub fn library_counts(
    connection: &mut SqliteConnection,
    user_id: i32,
    provider: ProviderId,
) -> QueryResult<LibraryCounts> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        albums: i64,
        #[diesel(sql_type = BigInt)]
        tracks: i64,
    }
    let row: Row = diesel::sql_query(
        "SELECT
           COALESCE(SUM(CASE WHEN kind = 'album' THEN 1 ELSE 0 END), 0) AS albums,
           COALESCE(SUM(CASE WHEN kind = 'track' THEN 1 ELSE 0 END), 0) AS tracks
         FROM provider_library_item WHERE user_id = ? AND provider = ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<Text, _>(provider.as_str())
    .get_result(connection)?;
    Ok(LibraryCounts {
        albums: row.albums,
        tracks: row.tracks,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sqlite::SqliteConnection;
    use diesel_migrations::MigrationHarness;

    use super::{
        AccountStatus, ItemOrigin, LibraryQuery, LibrarySort, NewLibraryItem, account, apply_sync,
        item_ids, library_counts, list_items, remove_account, save_credentials, save_session,
        set_status, sign_out, upsert_item,
    };
    use crate::providers::{ArtistRef, ItemKind, ProviderAlbum, ProviderId, StreamQuality};

    pub(crate) fn database() -> SqliteConnection {
        let mut connection = SqliteConnection::establish(":memory:").expect("database");
        connection
            .run_pending_migrations(crate::persistence::connection::MIGRATIONS)
            .expect("migrations");
        connection
            .batch_execute(
                "PRAGMA foreign_keys = ON;
                 INSERT INTO user(id, username, password) VALUES (1, 'ada', 'x'), (2, 'grace', 'x');",
            )
            .expect("users");
        connection
    }

    pub(crate) fn album(
        provider: ProviderId,
        raw: &str,
        title: &str,
        artist: &str,
    ) -> ProviderAlbum {
        ProviderAlbum {
            id: format!("{}:{raw}", provider.as_str()),
            provider,
            title: title.into(),
            version: None,
            artist: ArtistRef {
                id: format!("{}:9", provider.as_str()),
                name: artist.into(),
            },
            artists: Vec::new(),
            cover_url: None,
            release_date: Some("2024-01-01".into()),
            track_count: 3,
            duration: 600.0,
            explicit: false,
            format: None,
            label: None,
            genre: None,
            copyright: None,
            web_url: None,
            tracks: Vec::new(),
            in_library: false,
        }
    }

    #[test]
    fn credential_changes_sign_out_and_status_transitions_persist() {
        let mut connection = database();
        save_credentials(
            &mut connection,
            1,
            ProviderId::Tidal,
            b"sealed",
            StreamQuality::Lossless,
        )
        .expect("credentials");
        save_session(
            &mut connection,
            1,
            ProviderId::Tidal,
            b"session",
            "ada",
            Some("HiFi"),
        )
        .expect("session");
        let row = account(&mut connection, 1, ProviderId::Tidal)
            .expect("load")
            .expect("row");
        assert_eq!(row.status(), AccountStatus::SignedIn);
        assert_eq!(row.account_name.as_deref(), Some("ada"));

        set_status(
            &mut connection,
            1,
            ProviderId::Tidal,
            AccountStatus::Expired,
            Some("again"),
        )
        .expect("expire");
        assert_eq!(
            account(&mut connection, 1, ProviderId::Tidal)
                .expect("load")
                .expect("row")
                .status(),
            AccountStatus::Expired
        );

        save_credentials(
            &mut connection,
            1,
            ProviderId::Tidal,
            b"other",
            StreamQuality::Lossless,
        )
        .expect("new credentials");
        let row = account(&mut connection, 1, ProviderId::Tidal)
            .expect("load")
            .expect("row");
        assert_eq!(row.status(), AccountStatus::Configured);
        assert!(row.sealed_session.is_none());
        assert!(
            account(&mut connection, 2, ProviderId::Tidal)
                .expect("other user")
                .is_none()
        );

        sign_out(&mut connection, 1, ProviderId::Tidal).expect("sign out");
        assert!(
            account(&mut connection, 1, ProviderId::Tidal)
                .expect("load")
                .is_some()
        );
    }

    #[test]
    fn library_items_are_private_searchable_and_removed_with_the_account() {
        let mut connection = database();
        save_credentials(
            &mut connection,
            1,
            ProviderId::Qobuz,
            b"sealed",
            StreamQuality::HiResMax,
        )
        .expect("credentials");
        for (provider, raw, title, artist) in [
            (ProviderId::Qobuz, "q1", "Glass Orchard", "Ilse Marten"),
            (
                ProviderId::Qobuz,
                "q2",
                "Brass for Late Trains",
                "Northbound Brass",
            ),
            (ProviderId::Tidal, "7", "Harbor Lights", "Mara Vell"),
        ] {
            upsert_item(
                &mut connection,
                1,
                &NewLibraryItem::album(&album(provider, raw, title, artist)),
                ItemOrigin::Parson,
                10,
            )
            .expect("upsert");
        }
        let query = |provider, text| LibraryQuery {
            provider,
            kind: ItemKind::Album,
            text,
            sort: LibrarySort::Title,
            offset: 0,
            limit: 50,
        };
        let (all, total) = list_items(&mut connection, 1, &query(None, None)).expect("all");
        assert_eq!(total, 3);
        assert_eq!(all[0].title, "Brass for Late Trains");
        let (qobuz, _) =
            list_items(&mut connection, 1, &query(Some(ProviderId::Qobuz), None)).expect("qobuz");
        assert_eq!(qobuz.len(), 2);
        let (found, _) =
            list_items(&mut connection, 1, &query(None, Some("orchard marten"))).expect("search");
        assert_eq!(found.len(), 1);
        let (missing, _) =
            list_items(&mut connection, 1, &query(None, Some("orchard zz"))).expect("missing");
        assert!(missing.is_empty(), "every search word must match");
        let (other_user, _) = list_items(&mut connection, 2, &query(None, None)).expect("private");
        assert!(other_user.is_empty());
        assert!(
            item_ids(&mut connection, 1, None, ItemKind::Album)
                .expect("ids")
                .contains("tidal:7")
        );

        remove_account(&mut connection, 1, ProviderId::Qobuz).expect("remove");
        assert_eq!(
            library_counts(&mut connection, 1, ProviderId::Qobuz)
                .expect("counts")
                .albums,
            0
        );
        assert_eq!(
            library_counts(&mut connection, 1, ProviderId::Tidal)
                .expect("counts")
                .albums,
            1
        );
    }

    #[test]
    fn library_sync_adds_new_favorites_and_prunes_removed_ones() {
        let mut connection = database();
        upsert_item(
            &mut connection,
            1,
            &NewLibraryItem::album(&album(ProviderId::Tidal, "1", "Old", "A")),
            ItemOrigin::Parson,
            1,
        )
        .expect("seed");
        let fetched = vec![
            NewLibraryItem::album(&album(ProviderId::Tidal, "2", "New", "B")),
            NewLibraryItem::album(&album(ProviderId::Tidal, "3", "Newer", "C")),
        ];
        let (added, removed) = apply_sync(
            &mut connection,
            1,
            ProviderId::Tidal,
            ItemKind::Album,
            &fetched,
            true,
        )
        .expect("sync");
        assert_eq!((added, removed), (2, 1));
        let ids =
            item_ids(&mut connection, 1, Some(ProviderId::Tidal), ItemKind::Album).expect("ids");
        assert!(ids.contains("2") && ids.contains("3") && !ids.contains("1"));
    }

    #[test]
    fn deleting_a_user_removes_their_streaming_data() {
        let mut connection = database();
        save_credentials(
            &mut connection,
            2,
            ProviderId::Tidal,
            b"sealed",
            StreamQuality::Lossless,
        )
        .expect("credentials");
        upsert_item(
            &mut connection,
            2,
            &NewLibraryItem::album(&album(ProviderId::Tidal, "1", "Old", "A")),
            ItemOrigin::Parson,
            1,
        )
        .expect("item");
        connection
            .batch_execute("DELETE FROM user WHERE id = 2")
            .expect("delete user");
        assert!(
            account(&mut connection, 2, ProviderId::Tidal)
                .expect("load")
                .is_none()
        );
        assert_eq!(
            library_counts(&mut connection, 2, ProviderId::Tidal)
                .expect("counts")
                .albums,
            0
        );
    }
}
