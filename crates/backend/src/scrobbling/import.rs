//! Imports a profile's listening history from Last.fm or ListenBrainz.
//!
//! Remote listens are matched to songs in the library with the shared
//! matching engine; only confident matches are kept. Matched listens land in
//! the profile's listening history with their original timestamps and feed
//! its radio taste and recommendations, so a new Parson server already
//! knows what each person likes.

use std::collections::HashMap;
use std::sync::Arc;

use super::hub::{HubError, ScrobbleHub};
use super::store::{self, ImportStatus};
use super::{Service, ServiceError, lastfm, listenbrainz, now_unix};
use crate::library::matching::{MatchIndex, MatchStatus, SourceTrack};
use crate::library::state::LibraryCache;

/// Most listens one import reads (the history cap per profile).
pub const MAX_IMPORTED_LISTENS: usize = 20_000;
const LASTFM_PAGE_SIZE: u32 = 200;
const LISTENBRAINZ_PAGE_SIZE: u32 = 1000;

/// A listen read from a service.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteListen {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub listened_at: i64,
}

/// Matches remote listens to library songs, remembering each answer since
/// the same track recurs throughout a history.
pub struct Matcher {
    index: MatchIndex,
    known: HashMap<(String, String, String), Option<String>>,
}

impl Matcher {
    pub fn new(cache: &LibraryCache) -> Self {
        Self {
            index: MatchIndex::from_artists(&cache.artists),
            known: HashMap::new(),
        }
    }

    pub fn song_for(&mut self, listen: &RemoteListen) -> Option<String> {
        let key = (
            listen.artist.to_lowercase(),
            listen.track.to_lowercase(),
            listen.album.clone().unwrap_or_default().to_lowercase(),
        );
        if let Some(known) = self.known.get(&key) {
            return known.clone();
        }
        let result = self.index.match_track(&SourceTrack {
            title: listen.track.clone(),
            artists: vec![listen.artist.clone()],
            album: listen.album.clone(),
            ..SourceTrack::default()
        });
        let song = (result.status == MatchStatus::Matched)
            .then_some(result.song_id)
            .flatten();
        self.known.insert(key, song.clone());
        song
    }
}

fn status(
    state: &str,
    fetched: usize,
    matched: usize,
    imported: usize,
    started_at: i64,
) -> ImportStatus {
    ImportStatus {
        status: state.to_string(),
        fetched: fetched as i64,
        matched: matched as i64,
        imported: imported as i64,
        error: None,
        started_at,
        finished_at: None,
    }
}

/// Starts an import in the background. Fails at once when the profile is
/// not connected or an import is already running.
pub async fn start(
    hub: Arc<ScrobbleHub>,
    user_id: i32,
    service: Service,
    cache: Arc<LibraryCache>,
) -> Result<ImportStatus, HubError> {
    let (row, secret) = hub.active_connection(user_id, service).await?;
    if service == Service::Lastfm && hub.lastfm_credentials().await?.is_none() {
        return Err(HubError::NotConfigured);
    }
    if !hub.begin_import(user_id, service) {
        return Err(HubError::Busy);
    }
    let started = status("running", 0, 0, 0, now_unix());
    let initial = started.clone();
    if let Err(error) = hub
        .db(move |connection| store::write_import_status(connection, user_id, service, &initial))
        .await
    {
        hub.end_import(user_id, service);
        return Err(error);
    }
    let account = row.account_name.clone();
    let task_hub = hub.clone();
    tokio::spawn(async move {
        let result = run(&task_hub, user_id, service, &account, &secret, cache).await;
        let mut finished = match result {
            Ok(status) => status,
            Err((mut status, error)) => {
                status.status = "failed".into();
                status.error = Some(error);
                status
            }
        };
        finished.finished_at = Some(now_unix());
        if let Err(error) = task_hub
            .db(move |connection| {
                store::write_import_status(connection, user_id, service, &finished)
            })
            .await
        {
            tracing::warn!(%error, "could not record the history import result");
        }
        task_hub.end_import(user_id, service);
    });
    Ok(started)
}

async fn run(
    hub: &ScrobbleHub,
    user_id: i32,
    service: Service,
    account: &str,
    secret: &str,
    cache: Arc<LibraryCache>,
) -> Result<ImportStatus, (ImportStatus, String)> {
    let started_at = now_unix();
    let index_cache = cache.clone();
    let mut matcher = tokio::task::spawn_blocking(move || Matcher::new(&index_cache))
        .await
        .map_err(|error| (status("failed", 0, 0, 0, started_at), error.to_string()))?;
    let (mut fetched, mut matched, mut imported) = (0_usize, 0_usize, 0_usize);
    let mut lastfm_page = 1_u32;
    let mut max_ts: Option<i64> = None;
    let credentials = match service {
        Service::Lastfm => hub
            .lastfm_credentials()
            .await
            .ok()
            .flatten()
            .map(|(credentials, _)| credentials),
        Service::Listenbrainz => None,
    };
    loop {
        let page: Result<(Vec<RemoteListen>, bool), ServiceError> = match service {
            Service::Lastfm => {
                let Some(credentials) = credentials.as_ref() else {
                    return Err((
                        status("failed", fetched, matched, imported, started_at),
                        "This server has no Last.fm API key.".into(),
                    ));
                };
                lastfm::Client {
                    transport: hub.transport(),
                    base: hub.lastfm_api(),
                    credentials,
                }
                .recent_tracks(account, lastfm_page, LASTFM_PAGE_SIZE)
                .await
                .map(|page| {
                    let more = page.page < page.total_pages;
                    lastfm_page = page.page + 1;
                    (
                        page.listens
                            .into_iter()
                            .map(|listen| RemoteListen {
                                artist: listen.artist,
                                track: listen.track,
                                album: listen.album,
                                listened_at: listen.listened_at,
                            })
                            .collect(),
                        more,
                    )
                })
            }
            Service::Listenbrainz => listenbrainz::Client {
                transport: hub.transport(),
                base: hub.listenbrainz_api(),
            }
            .listens(Some(secret), account, max_ts, LISTENBRAINZ_PAGE_SIZE)
            .await
            .map(|listens| {
                let oldest = listens.iter().map(|listen| listen.listened_at).min();
                let more = oldest.is_some() && oldest != max_ts;
                max_ts = oldest;
                (
                    listens
                        .into_iter()
                        .map(|listen| RemoteListen {
                            artist: listen.artist,
                            track: listen.track,
                            album: listen.album,
                            listened_at: listen.listened_at,
                        })
                        .collect(),
                    more,
                )
            }),
        };
        let (listens, more) = page.map_err(|error| {
            (
                status("failed", fetched, matched, imported, started_at),
                error.message().to_string(),
            )
        })?;
        if listens.is_empty() {
            break;
        }
        fetched += listens.len();
        let songs = listens
            .iter()
            .filter_map(|listen| Some((matcher.song_for(listen)?, listen.listened_at)))
            .collect::<Vec<_>>();
        matched += songs.len();
        let page_cache = cache.clone();
        let added = hub
            .db(move |connection| {
                crate::recommendation::import_listens(connection, user_id, &songs, &page_cache)
            })
            .await
            .map_err(|error| {
                (
                    status("failed", fetched, matched, imported, started_at),
                    error.to_string(),
                )
            })?;
        imported += added;
        let progress = status("running", fetched, matched, imported, started_at);
        let _ = hub
            .db(move |connection| {
                store::write_import_status(connection, user_id, service, &progress)
            })
            .await;
        if !more || fetched >= MAX_IMPORTED_LISTENS {
            break;
        }
    }
    Ok(status("done", fetched, matched, imported, started_at))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::{Album, Artist, Song};
    use crate::scrobbling::hub::tests::{fixtures, hub_with, pool};
    use diesel::RunQueryDsl;
    use diesel::sql_types::BigInt;

    pub(crate) fn library() -> Arc<LibraryCache> {
        let song = |id: &str, name: &str, number: u16| Song {
            id: id.into(),
            name: name.into(),
            artist: "Ilse Marten".into(),
            track_number: number,
            duration: 214.0,
            path: format!("/music/Ilse Marten/Glass Orchard/{number:02} {name}.flac"),
            ..Song::default()
        };
        let artists = vec![Artist {
            id: "artist-ilse".into(),
            name: "Ilse Marten".into(),
            albums: vec![Album {
                id: "album-glass".into(),
                name: "Glass Orchard".into(),
                songs: vec![
                    song("song-lanterns", "Lanterns", 3),
                    song("song-copper", "Copper Sky", 4),
                ],
                ..Album::default()
            }],
            ..Artist::default()
        }];
        let mut cache = LibraryCache::from_artists(artists);
        for song in ["song-lanterns", "song-copper"] {
            cache
                .song_map
                .insert(song.into(), ("artist-ilse".into(), "album-glass".into()));
        }
        Arc::new(cache)
    }

    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    async fn wait_for_import(hub: &ScrobbleHub, user_id: i32, service: Service) -> ImportStatus {
        for _ in 0..200 {
            let status = hub
                .db(move |connection| store::import_status(connection, user_id, service))
                .await
                .unwrap()
                .unwrap();
            if status.status != "running" {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("import did not finish");
    }

    #[actix_web::test]
    async fn last_fm_history_is_matched_deduplicated_and_feeds_taste() {
        let pool = pool();
        let hub = Arc::new(hub_with(pool.clone(), fixtures()));
        let (url, _) = hub
            .start_lastfm_authorization(1, "http://localhost:1993")
            .await
            .unwrap();
        let state = url.split("state%3D").nth(1).unwrap().to_string();
        hub.complete_lastfm_authorization(&state, "approved-token")
            .await
            .unwrap();

        start(hub.clone(), 1, Service::Lastfm, library())
            .await
            .unwrap();
        // Only one import per profile and service at a time.
        assert!(!hub.begin_import(1, Service::Lastfm));
        let done = wait_for_import(&hub, 1, Service::Lastfm).await;
        assert_eq!(done.status, "done", "{:?}", done.error);
        // Two pages: 3 dated listens (the "now playing" row is skipped) + 1.
        assert_eq!(done.fetched, 4);
        // "Copper Sky (2021 Remaster)" still matches; the unknown song not.
        assert_eq!(done.matched, 3);
        assert_eq!(done.imported, 3);

        let mut connection = pool.get().unwrap();
        let history = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM listen_history_item WHERE user_id = 1
               AND listened_at = datetime(1699990000, 'unixepoch')",
        )
        .get_result::<Count>(&mut connection)
        .unwrap();
        assert_eq!(history.count, 1);
        let preference = diesel::sql_query(
            "SELECT CAST(qualified_plays AS BIGINT) AS count FROM user_track_preference
             WHERE user_id = 1 AND song_id = 'song-lanterns'",
        )
        .get_result::<Count>(&mut connection)
        .unwrap();
        assert_eq!(preference.count, 2);
        drop(connection);

        // A second import adds nothing new.
        start(hub.clone(), 1, Service::Lastfm, library())
            .await
            .unwrap();
        let again = wait_for_import(&hub, 1, Service::Lastfm).await;
        assert_eq!(again.imported, 0);
        assert_eq!(again.matched, 3);
    }

    #[actix_web::test]
    async fn listenbrainz_history_pages_by_timestamp() {
        let pool = pool();
        let transport = fixtures();
        let hub = Arc::new(hub_with(pool.clone(), transport.clone()));
        hub.connect_listenbrainz(1, "6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11")
            .await
            .unwrap();
        start(hub.clone(), 1, Service::Listenbrainz, library())
            .await
            .unwrap();
        let done = wait_for_import(&hub, 1, Service::Listenbrainz).await;
        assert_eq!(done.status, "done", "{:?}", done.error);
        assert_eq!(done.fetched, 3);
        assert_eq!(done.matched, 2);
        assert_eq!(done.imported, 2);
        let pages = transport
            .requests()
            .into_iter()
            .filter(|request| request.url.ends_with("/listens"))
            .collect::<Vec<_>>();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].query_value("max_ts"), Some("1699900000"));
        // Another profile's history is untouched.
        let mut connection = pool.get().unwrap();
        let other = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM listen_history_item WHERE user_id = 2",
        )
        .get_result::<Count>(&mut connection)
        .unwrap();
        assert_eq!(other.count, 0);
    }

    #[actix_web::test]
    async fn import_needs_a_connection() {
        let hub = Arc::new(hub_with(pool(), fixtures()));
        assert!(matches!(
            start(hub, 1, Service::Listenbrainz, library()).await,
            Err(HubError::NotConnected)
        ));
    }
}
