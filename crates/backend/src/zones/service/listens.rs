//! Listening history for zones the server plays itself.
//!
//! Web, desktop and phone zones report their own plays under the profile
//! signed in on that device. Server-driven zones (network renderers, the
//! server's audio devices) have no device session, so the zone remembers
//! which profile last started or steered playback, as Roon does, and the
//! server records that profile's plays: "now playing", the qualified play
//! after half the track or four minutes (history, Radio taste,
//! recommendations), and the scrobbles that follow from them.

use std::sync::Arc;

use super::*;
use crate::recommendation::{PlaybackEventRequest, record_playback_event};
use crate::scrobbling::{ScrobbleHub, rules};

/// Largest position jump between two polls still counted as listening;
/// anything bigger is a seek.
const MAX_LISTEN_STEP_MS: i64 = 15_000;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ListenProgress {
    serial: u64,
    song_id: String,
    started_at: i64,
    listened_ms: i64,
    last_position_ms: i64,
    announced: bool,
    qualified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListenEvent {
    Started,
    Qualified,
}

impl ListenEvent {
    fn event_type(self) -> &'static str {
        match self {
            Self::Started => "play_started",
            Self::Qualified => "qualified_play",
        }
    }
}

/// Folds one renderer observation into the zone's listen. Returns the
/// events it completes. Only time actually played counts: pauses, seeks and
/// skips do not.
pub(super) fn observe_listen(
    progress: &mut Option<ListenProgress>,
    serial: u64,
    song_id: &str,
    position_ms: i64,
    playing: bool,
    duration_seconds: f64,
    now_seconds: i64,
) -> Vec<ListenEvent> {
    let position_ms = position_ms.max(0);
    let fresh = progress
        .as_ref()
        .is_none_or(|listen| listen.serial != serial || listen.song_id != song_id);
    if fresh {
        *progress = Some(ListenProgress {
            serial,
            song_id: song_id.to_string(),
            started_at: now_seconds - position_ms / 1000,
            listened_ms: 0,
            last_position_ms: position_ms,
            announced: false,
            qualified: false,
        });
    }
    let Some(listen) = progress.as_mut() else {
        return Vec::new();
    };
    let mut events = Vec::new();
    if !playing {
        listen.last_position_ms = position_ms;
        return events;
    }
    if !listen.announced {
        listen.announced = true;
        events.push(ListenEvent::Started);
    }
    let step = position_ms - listen.last_position_ms;
    if (0..=MAX_LISTEN_STEP_MS).contains(&step) {
        listen.listened_ms += step;
    }
    listen.last_position_ms = position_ms;
    if !listen.qualified && rules::qualifies(duration_seconds, listen.listened_ms as f64 / 1000.0) {
        listen.qualified = true;
        events.push(ListenEvent::Qualified);
    }
    events
}

impl ZoneService {
    /// Lets zone plays reach scrobbling.
    pub fn set_scrobbling(&self, hub: Arc<ScrobbleHub>) {
        let _ = self.scrobbling.set(hub);
    }

    /// Remembers who is listening in a server-driven zone: the profile
    /// that loaded it or steered playback last.
    pub(super) fn note_listener(&self, zone_id: &str, user_id: i32, command: &ZoneCommand) {
        if matches!(
            command,
            ZoneCommand::Load { .. }
                | ZoneCommand::Play
                | ZoneCommand::PlayPause
                | ZoneCommand::Next
                | ZoneCommand::Previous
                | ZoneCommand::Jump { .. }
                | ZoneCommand::AddToQueue { .. }
        ) {
            self.lock().listeners.insert(zone_id.to_string(), user_id);
        }
    }

    /// The profile listening in a zone, if one started it.
    pub fn zone_listener(&self, zone_id: &str) -> Option<i32> {
        self.lock().listeners.get(zone_id).copied()
    }

    /// Records the listen of the track a server-driven zone is playing.
    pub(super) fn track_zone_listen(
        &self,
        zone_id: &str,
        zone_name: &str,
        serial: u64,
        song_id: &str,
        position_ms: i64,
        playing: bool,
        cache: Option<&Arc<LibraryCache>>,
    ) {
        let Some(cache) = cache else {
            return;
        };
        let duration = cache.song(song_id).map_or(0.0, |song| song.duration);
        let (events, listener, started_at) = {
            let mut state = self.lock();
            let Some(listener) = state.listeners.get(zone_id).copied() else {
                return;
            };
            let progress = state.zone_listens.entry(zone_id.to_string()).or_default();
            let events = observe_listen(
                progress,
                serial,
                song_id,
                position_ms,
                playing,
                duration,
                chrono::Utc::now().timestamp(),
            );
            let started_at = progress.as_ref().map_or(0, |listen| listen.started_at);
            (events, listener, started_at)
        };
        if events.is_empty() {
            return;
        }
        let Some(pool) = self.pool.clone() else {
            return;
        };
        let hub = self.scrobbling.get().cloned();
        let cache = cache.clone();
        let zone_id = zone_id.to_string();
        let zone_name = Some(zone_name.trim().to_string()).filter(|name| !name.is_empty());
        let song_id = song_id.to_string();
        tokio::spawn(async move {
            for event in events {
                let request = PlaybackEventRequest {
                    event_key: format!(
                        "zone:{zone_id}:{song_id}:{started_at}:{}",
                        event.event_type()
                    ),
                    song_id: song_id.clone(),
                    event_type: event.event_type().to_string(),
                    session_id: Some(format!("zone:{zone_id}")),
                    queue_id: None,
                    source: Some("zone".to_string()),
                    position_seconds: Some((position_ms.max(0) / 1000) as f64),
                    duration_seconds: Some(duration),
                    zone_id: Some(zone_id.clone()),
                    zone_name: zone_name.clone(),
                    ..Default::default()
                };
                let record_pool = pool.clone();
                let record_cache = cache.clone();
                let stored = request.clone();
                let accepted = tokio::task::spawn_blocking(move || {
                    record_playback_event(listener, &stored, &record_cache, &record_pool)
                        .map(|result| result.accepted)
                        .map_err(|error| error.to_string())
                })
                .await;
                match accepted {
                    Ok(Ok(true)) => {
                        if let Some(hub) = &hub
                            && let Err(error) = hub.observe(listener, request, cache.clone()).await
                        {
                            tracing::warn!(%error, "could not scrobble a zone play");
                        }
                    }
                    Ok(Ok(false)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, zone_id, "could not record a zone play")
                    }
                    Err(error) => tracing::warn!(%error, "zone play recorder stopped"),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_listen_qualifies_after_half_the_track_of_actual_playing() {
        let mut progress = None;
        // Paused at the start: nothing yet.
        assert!(observe_listen(&mut progress, 1, "a", 0, false, 200.0, 1_000).is_empty());
        assert_eq!(
            observe_listen(&mut progress, 1, "a", 1_000, true, 200.0, 1_001),
            vec![ListenEvent::Started]
        );
        let mut events = Vec::new();
        for second in 2..=101 {
            events.extend(observe_listen(
                &mut progress,
                1,
                "a",
                second * 1_000,
                true,
                200.0,
                1_000 + second,
            ));
        }
        assert_eq!(events, vec![ListenEvent::Qualified]);
        assert_eq!(progress.as_ref().unwrap().started_at, 1_000);
        // Further polls of the same play add nothing.
        assert!(observe_listen(&mut progress, 1, "a", 150_000, true, 200.0, 1_150).is_empty());
    }

    #[actix_web::test]
    async fn plays_in_a_server_driven_zone_count_for_the_profile_that_started_them() {
        use crate::scrobbling::hub::tests::{fixtures, hub_with, pool};
        use diesel::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }
        let count = |pool: &DbPool, sql: &str| {
            diesel::sql_query(sql)
                .get_result::<Count>(&mut pool.get().unwrap())
                .unwrap()
                .count
        };

        let pool = pool();
        let hub = Arc::new(hub_with(pool.clone(), fixtures()));
        hub.connect_listenbrainz(2, "6c4b6d2e-6f4c-4d2b-9a43-0f6f7a2b9c11")
            .await
            .unwrap();
        let cache = crate::scrobbling::import::tests::library();
        let service = ZoneService::new(None, Some(pool.clone()));
        service.set_scrobbling(hub);
        let zone = "upnp:living-room";

        // Nobody started it: nothing is recorded.
        service.track_zone_listen(
            zone,
            "Living Room",
            1,
            "song-lanterns",
            0,
            true,
            Some(&cache),
        );
        assert!(service.lock().zone_listens.is_empty());

        // Grace (profile 2) starts it from her phone.
        service.note_listener(zone, 2, &ZoneCommand::Play);
        service.note_listener(
            zone,
            1,
            &ZoneCommand::SetMute {
                muted: true,
                member_id: None,
            },
        );
        assert_eq!(service.zone_listener(zone), Some(2));
        for second in 0..=110 {
            service.track_zone_listen(
                zone,
                "Living Room",
                1,
                "song-lanterns",
                second * 1_000,
                true,
                Some(&cache),
            );
        }
        let mut recorded = false;
        for _ in 0..200 {
            if count(
                &pool,
                "SELECT COUNT(*) AS count FROM scrobble_queue WHERE user_id = 2",
            ) == 1
            {
                recorded = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(recorded, "the qualified play reached the scrobble queue");
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) AS count FROM listen_history_item WHERE user_id = 2"
            ),
            1
        );
        // The history names the zone the play happened in.
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) AS count FROM listen_history_item
                 WHERE user_id = 2 AND zone_id = 'upnp:living-room'
                   AND zone_name = 'Living Room' AND source = 'zone'"
            ),
            1
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) AS count FROM playback_event
                 WHERE user_id = 2 AND session_id = 'zone:upnp:living-room'"
            ),
            2
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) AS count FROM listen_history_item WHERE user_id = 1"
            ),
            0
        );
    }

    #[test]
    fn seeking_ahead_does_not_count_as_listening() {
        let mut progress = None;
        observe_listen(&mut progress, 7, "a", 0, true, 300.0, 0);
        // Jumped to the end.
        assert!(observe_listen(&mut progress, 7, "a", 290_000, true, 300.0, 2).is_empty());
        assert_eq!(progress.as_ref().unwrap().listened_ms, 0);
    }

    #[test]
    fn a_new_track_starts_a_new_listen_and_short_tracks_never_qualify() {
        let mut progress = None;
        observe_listen(&mut progress, 1, "a", 0, true, 300.0, 0);
        assert_eq!(
            observe_listen(&mut progress, 2, "b", 0, true, 25.0, 10),
            vec![ListenEvent::Started]
        );
        let mut events = Vec::new();
        for second in 1..=25 {
            events.extend(observe_listen(
                &mut progress,
                2,
                "b",
                second * 1_000,
                true,
                25.0,
                10 + second,
            ));
        }
        assert!(events.is_empty());
    }
}
