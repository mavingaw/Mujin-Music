//! Server-driven zones: UPnP/DLNA renderers and the audio devices of the
//! machine running Parson. The server holds their queue and plays it one
//! track at a time (with the next track queued for gapless playback),
//! following each renderer's state to move on when a track ends.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;
use crate::outputs::renderer::RendererSettings;
use crate::outputs::upnp::RENDER_MEDIA_PATH;
use crate::outputs::{
    FormatLimits, Outputs, PlayState, Renderer, RendererInfo, RendererState, SourceFormat,
    TrackRequest, describe_source, format_rate,
};

/// What the listener asked a server-driven zone to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Desired {
    Playing,
    Paused,
    Stopped,
}

pub(super) struct HostedQueue {
    song_ids: Vec<String>,
    current: usize,
    shuffle: bool,
    original: Option<Vec<String>>,
    repeat: RepeatMode,
    desired: Desired,
    /// Serial of the track loaded as current, and of the queued next one.
    current_serial: u64,
    next: Option<(usize, u64)>,
    /// The renderer was seen playing the current track.
    started: bool,
    /// Where to resume after a stop.
    resume_ms: i64,
}

impl HostedQueue {
    /// The queue position after the current one, honouring repeat.
    fn following(&self) -> Option<usize> {
        match self.repeat {
            RepeatMode::One => Some(self.current),
            _ if self.current + 1 < self.song_ids.len() => Some(self.current + 1),
            RepeatMode::All if !self.song_ids.is_empty() => Some(0),
            _ => None,
        }
    }
}

/// What a signed render URL is signed for.
pub fn render_scope(zone_id: &str) -> String {
    format!("render:{zone_id}")
}

/// What the media endpoint applies to a server-driven zone's stream.
pub struct HostedStream {
    pub limits: FormatLimits,
    pub leveling: Option<StreamLeveling>,
    pub dsp: ZoneDsp,
}

fn source_format(song: &crate::domain::Song) -> SourceFormat {
    let audio = song.audio.clone().unwrap_or_default();
    let extension = std::path::Path::new(&song.path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    SourceFormat {
        codec: audio.codec.or_else(|| match extension.as_deref() {
            Some("flac") => Some("flac".to_string()),
            Some("wav") => Some("pcm".to_string()),
            Some("mp3") => Some("mp3".to_string()),
            Some("m4a" | "aac" | "mp4") => Some("aac".to_string()),
            _ => None,
        }),
        sample_rate: audio.sample_rate,
        bit_depth: audio.bit_depth,
        channels: audio.channels,
    }
}

/// The settings a renderer applies, from the zone's stored settings.
fn renderer_settings(info: &RendererInfo, settings: &ZoneSettings) -> RendererSettings {
    let volume_mode = settings
        .volume_mode
        .filter(|mode| info.volume_modes.contains(mode))
        .or_else(|| info.volume_modes.first().copied())
        .unwrap_or(VolumeMode::Fixed);
    RendererSettings {
        exclusive: settings.exclusive,
        volume_mode,
        limits: info
            .limits
            .capped(settings.max_sample_rate, settings.max_bit_depth),
    }
}

impl ZoneService {
    /// Hands the service its UPnP renderers and audio devices.
    pub fn set_outputs(&self, outputs: Arc<Outputs>) {
        let _ = self.outputs.set(outputs);
    }

    pub fn outputs(&self) -> Option<&Arc<Outputs>> {
        self.outputs.get()
    }

    /// Lets server-driven zones read the library when they move on.
    pub fn set_library(&self, library: Arc<crate::library::state::LibraryLifecycle>) {
        let _ = self.library.set(library);
    }

    async fn library_cache(&self) -> Option<Arc<LibraryCache>> {
        self.library.get()?.cache().await.ok()
    }

    fn renderer(&self, zone_id: &str) -> Option<Arc<dyn Renderer>> {
        self.outputs.get()?.renderer(zone_id)
    }

    pub(super) fn hosted_views(
        &self,
        state: &State,
        cache: Option<&LibraryCache>,
        now: i64,
    ) -> Vec<Zone> {
        let Some(outputs) = self.outputs.get() else {
            return Vec::new();
        };
        outputs
            .renderers()
            .iter()
            .map(|renderer| hosted_zone_view(state, renderer.as_ref(), cache, now))
            .collect()
    }

    /// Whether a signed render URL for `zone_id` may stream `song_id`, and
    /// how: the zone's format limits, leveling and DSP.
    pub fn hosted_stream(
        &self,
        zone_id: &str,
        song_id: &str,
        cache: &LibraryCache,
    ) -> Option<HostedStream> {
        let renderer = self.renderer(zone_id)?;
        let info = renderer.info();
        let state = self.lock();
        let queue = state.hosted.get(zone_id)?;
        let index = queue.song_ids.iter().position(|id| id == song_id)?;
        let settings = state.settings.get(zone_id).cloned().unwrap_or_default();
        Some(HostedStream {
            limits: renderer_settings(&info, &settings).limits,
            leveling: sonos_leveling(&settings, &queue.song_ids, index, cache),
            dsp: settings.dsp,
        })
    }

    /// Builds what a renderer needs for queue position `index`.
    async fn track_request(
        &self,
        zone_id: &str,
        renderer: &dyn Renderer,
        song_ids: &[String],
        index: usize,
        cache: &LibraryCache,
    ) -> Result<TrackRequest, ZoneError> {
        let song_id = song_ids
            .get(index)
            .ok_or(ZoneError::Invalid("invalid_queue_position"))?;
        let song = cache
            .song(song_id)
            .ok_or(ZoneError::Invalid("invalid_zone_queue"))?;
        let info = renderer.info();
        let settings = self
            .lock()
            .settings
            .get(zone_id)
            .cloned()
            .unwrap_or_default();
        let effective = renderer_settings(&info, &settings);
        let leveling = sonos_leveling(&settings, song_ids, index, cache);
        let duration_ms = (song.duration.max(0.0) * 1_000.0).round() as i64;
        let serial = self.track_serial.fetch_add(1, Ordering::Relaxed);
        let mut request = TrackRequest {
            serial,
            song_id: song_id.clone(),
            path: song.path.clone(),
            duration_ms,
            source: source_format(song),
            uri: String::new(),
            metadata: String::new(),
            mime_type: String::new(),
            gain_db: None,
            dsp: settings.dsp,
        };
        if !info.network {
            // The device decodes the file itself; leveling becomes a gain.
            if let Some(leveling) = leveling {
                request.gain_db = self
                    .loudness
                    .gain_for(
                        cache,
                        song_id,
                        leveling.by,
                        leveling.target_lufs,
                        leveling.adjustment_db,
                    )
                    .await;
            }
            return Ok(request);
        }
        let origin = renderer
            .media_origin(self.port())
            .ok_or_else(|| ZoneError::Device("no network route to the renderer".into()))?;
        let expires = chrono::Utc::now().timestamp() + MEDIA_URL_LIFETIME_SECONDS;
        let signature = crate::api::cast::sign_media_url(&render_scope(zone_id), song_id, expires);
        let uri = format!(
            "{origin}{RENDER_MEDIA_PATH}{}/stream?zone={}&expires={expires}&signature={signature}",
            crate::api::cast::encoded(song_id),
            crate::api::cast::encoded(zone_id),
        );
        let plan = crate::api::library::renderer_stream_plan(
            song,
            effective.limits,
            leveling.is_some() || settings.dsp.is_active(),
        );
        let album = cache
            .song_map
            .get(song_id)
            .and_then(|(_, album_id)| cache.album(album_id));
        let album_art_uri = album
            .filter(|album| !album.cover_url.trim().is_empty())
            .map(|album| {
                let image_signature = crate::api::image::sign_image_path(&album.cover_url, expires);
                format!(
                    "{origin}/media/images/{}?raw=true&expires={expires}&image_signature={image_signature}",
                    crate::api::cast::encoded(&album.cover_url)
                )
            });
        request.metadata = crate::sonos::didl::render(&crate::sonos::didl::DidlTrack {
            item_id: format!("parson:{song_id}"),
            title: song.name.clone(),
            artist: song.artist.clone(),
            album: album.map(|album| album.name.clone()).unwrap_or_default(),
            album_art_uri,
            duration_ms,
            uri: uri.clone(),
            mime_type: plan.mime_type.to_string(),
        });
        request.mime_type = plan.mime_type.to_string();
        request.uri = uri;
        if leveling.is_some() {
            // Measure ahead so the renderer does not wait when it fetches.
            self.loudness
                .measure_in_background(vec![(song_id.clone(), song.path.clone())]);
        }
        Ok(request)
    }

    /// Loads the queue's current track (and the next one) on the renderer.
    async fn hosted_start(
        &self,
        zone_id: &str,
        renderer: &dyn Renderer,
        cache: &LibraryCache,
        position_ms: i64,
        play: bool,
    ) -> Result<(), ZoneError> {
        let (song_ids, current, following) = {
            let state = self.lock();
            let queue = state.hosted.get(zone_id).ok_or(ZoneError::NotFound)?;
            (queue.song_ids.clone(), queue.current, queue.following())
        };
        let track = self
            .track_request(zone_id, renderer, &song_ids, current, cache)
            .await?;
        let next = match following {
            Some(index) => self
                .track_request(zone_id, renderer, &song_ids, index, cache)
                .await
                .ok()
                .map(|request| (index, request)),
            None => None,
        };
        {
            let mut state = self.lock();
            if let Some(queue) = state.hosted.get_mut(zone_id) {
                queue.current_serial = track.serial;
                queue.next = next
                    .as_ref()
                    .map(|(index, request)| (*index, request.serial));
                queue.started = false;
                queue.desired = if play {
                    Desired::Playing
                } else {
                    Desired::Paused
                };
            }
        }
        renderer
            .load(track, next.map(|(_, request)| request), position_ms, play)
            .await
            .map_err(ZoneError::Device)
    }

    /// Re-sends the track after the current one (queue edits, repeat,
    /// settings that change the stream).
    async fn hosted_send_next(
        &self,
        zone_id: &str,
        renderer: &dyn Renderer,
        cache: &LibraryCache,
    ) -> Result<(), ZoneError> {
        let Some((song_ids, following)) = ({
            let state = self.lock();
            state
                .hosted
                .get(zone_id)
                .filter(|queue| queue.desired != Desired::Stopped)
                .map(|queue| (queue.song_ids.clone(), queue.following()))
        }) else {
            return Ok(());
        };
        let next = match following {
            Some(index) => Some((
                index,
                self.track_request(zone_id, renderer, &song_ids, index, cache)
                    .await?,
            )),
            None => None,
        };
        if let Some(queue) = self.lock().hosted.get_mut(zone_id) {
            queue.next = next
                .as_ref()
                .map(|(index, request)| (*index, request.serial));
        }
        renderer
            .set_next(next.map(|(_, request)| request))
            .await
            .map_err(ZoneError::Device)
    }

    /// A remote-control command for a server-driven zone.
    pub(super) async fn hosted_command(
        &self,
        zone_id: &str,
        command: ZoneCommand,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        let renderer = self.renderer(zone_id).ok_or(ZoneError::NotFound)?;
        let info = renderer.info();
        let settings = self
            .lock()
            .settings
            .get(zone_id)
            .cloned()
            .unwrap_or_default();
        let effective = renderer_settings(&info, &settings);
        renderer.configure(effective);
        let owned;
        let cache = match cache {
            Some(cache) => Some(cache),
            None => {
                owned = self.library_cache().await;
                owned.as_deref()
            }
        };
        let need_cache = || cache.ok_or(ZoneError::Unsupported("library_not_ready"));
        let device = ZoneError::Device;
        let result = match command {
            ZoneCommand::Load {
                song_ids,
                current_position,
                position_ms,
                play,
            } => {
                let cache = need_cache()?;
                if song_ids.iter().any(|song_id| cache.song(song_id).is_none()) {
                    return Err(ZoneError::Invalid("invalid_zone_queue"));
                }
                self.lock().hosted.insert(
                    zone_id.to_string(),
                    HostedQueue {
                        song_ids,
                        current: current_position,
                        shuffle: false,
                        original: None,
                        repeat: RepeatMode::Off,
                        desired: if play {
                            Desired::Playing
                        } else {
                            Desired::Paused
                        },
                        current_serial: 0,
                        next: None,
                        started: false,
                        resume_ms: position_ms,
                    },
                );
                self.hosted_start(zone_id, renderer.as_ref(), cache, position_ms, play)
                    .await
            }
            ZoneCommand::Play => {
                let state = renderer.state();
                let resume = {
                    let mut zones = self.lock();
                    let queue = zones
                        .hosted
                        .get_mut(zone_id)
                        .ok_or(ZoneError::Unsupported("zone_has_no_parson_queue"))?;
                    let restart = state.state == PlayState::Stopped
                        || state.serial != Some(queue.current_serial)
                        || state.foreign.is_some();
                    queue.desired = Desired::Playing;
                    restart.then_some(queue.resume_ms)
                };
                match resume {
                    Some(position_ms) => {
                        self.hosted_start(
                            zone_id,
                            renderer.as_ref(),
                            need_cache()?,
                            position_ms,
                            true,
                        )
                        .await
                    }
                    None => renderer.play().await.map_err(device),
                }
            }
            ZoneCommand::Pause => {
                let position = renderer.state().position_ms;
                if let Some(queue) = self.lock().hosted.get_mut(zone_id) {
                    queue.desired = Desired::Paused;
                    queue.resume_ms = position;
                }
                renderer.pause().await.map_err(device)
            }
            ZoneCommand::PlayPause => {
                let playing = matches!(
                    renderer.state().state,
                    PlayState::Playing | PlayState::Transitioning
                );
                let next = if playing {
                    ZoneCommand::Pause
                } else {
                    ZoneCommand::Play
                };
                return Box::pin(self.hosted_command(zone_id, next, cache)).await;
            }
            ZoneCommand::Stop => {
                if let Some(queue) = self.lock().hosted.get_mut(zone_id) {
                    queue.desired = Desired::Stopped;
                    queue.resume_ms = 0;
                }
                renderer.stop().await.map_err(device)
            }
            ZoneCommand::Seek { position_ms } => {
                if let Some(queue) = self.lock().hosted.get_mut(zone_id) {
                    queue.resume_ms = position_ms;
                }
                renderer.seek(position_ms).await.map_err(device)
            }
            ZoneCommand::Next | ZoneCommand::Previous | ZoneCommand::Jump { .. } => {
                let position = renderer.state().position_ms;
                let (target, play) = {
                    let state = self.lock();
                    let queue = state
                        .hosted
                        .get(zone_id)
                        .ok_or(ZoneError::Unsupported("zone_has_no_parson_queue"))?;
                    let target = match command {
                        ZoneCommand::Next => match queue.repeat {
                            // Next skips even when repeating one track.
                            RepeatMode::One | RepeatMode::Off
                                if queue.current + 1 < queue.song_ids.len() =>
                            {
                                Some(queue.current + 1)
                            }
                            RepeatMode::All if !queue.song_ids.is_empty() => {
                                Some((queue.current + 1) % queue.song_ids.len())
                            }
                            _ => None,
                        },
                        ZoneCommand::Previous => {
                            if position > 3_000 || queue.current == 0 {
                                Some(queue.current)
                            } else {
                                Some(queue.current - 1)
                            }
                        }
                        ZoneCommand::Jump { queue_position } => {
                            if queue_position >= queue.song_ids.len() {
                                return Err(ZoneError::Invalid("invalid_queue_position"));
                            }
                            Some(queue_position)
                        }
                        _ => None,
                    };
                    (target, queue.desired != Desired::Paused)
                };
                let Some(target) = target else {
                    return Err(ZoneError::Invalid("end_of_queue"));
                };
                if let Some(queue) = self.lock().hosted.get_mut(zone_id) {
                    queue.current = target;
                    queue.resume_ms = 0;
                }
                self.hosted_start(zone_id, renderer.as_ref(), need_cache()?, 0, play)
                    .await
            }
            ZoneCommand::SetVolume {
                volume,
                above_comfort,
                ..
            } => {
                if effective.volume_mode == VolumeMode::Fixed {
                    return Err(ZoneError::Unsupported("fixed_volume"));
                }
                let level = settings
                    .limits()
                    .clamp(i16::from(volume), false, above_comfort);
                renderer.set_volume(level).await.map_err(device)
            }
            ZoneCommand::AdjustVolume { delta, .. } => {
                if effective.volume_mode == VolumeMode::Fixed {
                    return Err(ZoneError::Unsupported("fixed_volume"));
                }
                let current = renderer.state().volume;
                let level =
                    settings
                        .limits()
                        .clamp(i16::from(current) + i16::from(delta), true, true);
                renderer.set_volume(level).await.map_err(device)
            }
            ZoneCommand::SetMute { muted, .. } => renderer.set_mute(muted).await.map_err(device),
            ZoneCommand::SetRepeat { mode } => {
                self.lock()
                    .hosted
                    .get_mut(zone_id)
                    .ok_or(ZoneError::Unsupported("zone_has_no_parson_queue"))?
                    .repeat = mode;
                self.hosted_send_next(zone_id, renderer.as_ref(), need_cache()?)
                    .await
            }
            ZoneCommand::MoveQueueItem { .. }
            | ZoneCommand::RemoveQueueItem { .. }
            | ZoneCommand::ClearUpcoming
            | ZoneCommand::SetShuffle { .. } => {
                {
                    let mut state = self.lock();
                    let queue = state
                        .hosted
                        .get_mut(zone_id)
                        .ok_or(ZoneError::Unsupported("zone_has_no_parson_queue"))?;
                    let mut random = |bound: usize| rand::random_range(0..bound.max(1));
                    apply_queue_edit(
                        &mut queue.song_ids,
                        queue.current,
                        &mut queue.shuffle,
                        &mut queue.original,
                        &command,
                        &mut random,
                    )?;
                }
                self.hosted_send_next(zone_id, renderer.as_ref(), need_cache()?)
                    .await
            }
        };
        self.changed(None);
        result
    }

    /// Re-applies settings to a renderer and refreshes its next track (a
    /// new leveling, DSP or format limit applies from the next track).
    pub(super) async fn hosted_settings_changed(&self, zone_id: &str) {
        let Some(renderer) = self.renderer(zone_id) else {
            return;
        };
        let info = renderer.info();
        let settings = self
            .lock()
            .settings
            .get(zone_id)
            .cloned()
            .unwrap_or_default();
        renderer.configure(renderer_settings(&info, &settings));
        if let Some(cache) = self.library_cache().await
            && let Err(error) = self
                .hosted_send_next(zone_id, renderer.as_ref(), &cache)
                .await
        {
            tracing::warn!(%error, zone_id, "could not refresh the next track");
        }
    }

    /// Follows every server-driven zone: polls its renderer and moves the
    /// queue on when a track ends. Idle renderers are polled less often.
    pub async fn drive_hosted(&self, tick: u64) {
        let Some(outputs) = self.outputs.get().cloned() else {
            return;
        };
        let mut changed = false;
        let renderers: Vec<(Arc<dyn Renderer>, RendererInfo, bool)> = outputs
            .renderers()
            .into_iter()
            .map(|renderer| {
                let info = renderer.info();
                let active = self
                    .lock()
                    .hosted
                    .get(&info.id)
                    .is_some_and(|queue| queue.desired != Desired::Stopped);
                (renderer, info, active)
            })
            .collect();
        // Poll together, so one slow renderer does not hold up the others.
        futures::future::join_all(
            renderers
                .iter()
                .filter(|(_, _, active)| *active || tick.is_multiple_of(5))
                .map(|(renderer, _, _)| renderer.poll()),
        )
        .await;
        for (renderer, info, active) in renderers {
            if active {
                self.advance_hosted(&info.id, renderer.as_ref()).await;
            }
            let state = renderer.state();
            let fingerprint = format!(
                "{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{}",
                state.state,
                state.serial,
                state.foreign,
                state.volume,
                state.muted,
                state.reachable,
                state.stream.as_ref().map(|stream| stream.bit_perfect),
                info.name,
            );
            let mut zones = self.lock();
            if zones.hosted_seen.get(&info.id) != Some(&fingerprint) {
                zones.hosted_seen.insert(info.id.clone(), fingerprint);
                changed = true;
            }
        }
        if changed {
            self.changed(None);
        }
    }

    /// Moves a playing zone's queue along with its renderer.
    async fn advance_hosted(&self, zone_id: &str, renderer: &dyn Renderer) {
        enum Step {
            Nothing,
            /// The renderer moved on to the queued next track by itself.
            MovedOn,
            /// The track ended with nothing queued: start the new current one.
            Start,
        }
        let state: RendererState = renderer.state();
        let step = {
            let mut zones = self.lock();
            let Some(queue) = zones.hosted.get_mut(zone_id) else {
                return;
            };
            if queue.desired != Desired::Playing {
                Step::Nothing
            } else if state.foreign.is_some() {
                // Another app took the renderer over.
                queue.desired = Desired::Stopped;
                Step::Nothing
            } else if let Some((index, serial)) = queue.next
                && state.serial == Some(serial)
            {
                queue.current = index;
                queue.current_serial = serial;
                queue.next = None;
                queue.started = state.state == PlayState::Playing;
                Step::MovedOn
            } else if state.finished_serial == Some(queue.current_serial) {
                match queue.following() {
                    Some(index) => {
                        queue.current = index;
                        Step::Start
                    }
                    None => {
                        queue.desired = Desired::Stopped;
                        queue.resume_ms = 0;
                        Step::Nothing
                    }
                }
            } else if state.serial == Some(queue.current_serial) {
                if state.state == PlayState::Playing {
                    queue.started = true;
                    queue.resume_ms = state.position_ms;
                } else if state.state == PlayState::Stopped && queue.started {
                    // Stopped from the renderer's own remote mid-track.
                    queue.desired = Desired::Stopped;
                }
                Step::Nothing
            } else {
                Step::Nothing
            }
        };
        let Some(cache) = (match step {
            Step::Nothing => None,
            _ => self.library_cache().await,
        }) else {
            return;
        };
        let result = match step {
            Step::Nothing => Ok(()),
            Step::MovedOn => self.hosted_send_next(zone_id, renderer, &cache).await,
            Step::Start => self.hosted_start(zone_id, renderer, &cache, 0, true).await,
        };
        if let Err(error) = result {
            tracing::warn!(%error, zone_id, "server-driven zone could not move on");
        }
        self.changed(None);
    }
}

/// The zone of one renderer.
fn hosted_zone_view(
    state: &State,
    renderer: &dyn Renderer,
    cache: Option<&LibraryCache>,
    now: i64,
) -> Zone {
    let info = renderer.info();
    let renderer_state = renderer.state();
    let id = info.id.clone();
    let settings = state.settings.get(&id).cloned().unwrap_or_default();
    let effective = renderer_settings(&info, &settings);
    let queue = state.hosted.get(&id);
    let source = if renderer_state.foreign.is_some() {
        ZoneSource::External
    } else if queue.is_some_and(|queue| !queue.song_ids.is_empty()) {
        ZoneSource::Parson
    } else {
        ZoneSource::None
    };
    let current = queue.map(|queue| queue.current).unwrap_or(0);
    let current_song = queue.and_then(|queue| queue.song_ids.get(current));
    let now_playing = match source {
        ZoneSource::Parson => current_song.and_then(|song_id| now_playing_for(cache, song_id)),
        ZoneSource::External => Some(ZoneNowPlaying {
            song_id: None,
            title: renderer_state
                .foreign
                .clone()
                .unwrap_or_else(|| "Playing from another app".to_string()),
            artist: String::new(),
            album: String::new(),
            album_id: None,
            cover_url: None,
            duration_ms: renderer_state.duration_ms,
        }),
        ZoneSource::None => None,
    };
    let status = if !renderer_state.reachable {
        ZoneStatus::Unavailable
    } else {
        match renderer_state.state {
            PlayState::Playing => ZoneStatus::Playing,
            PlayState::Paused => ZoneStatus::Paused,
            PlayState::Transitioning => ZoneStatus::Buffering,
            PlayState::Stopped if source == ZoneSource::None => ZoneStatus::Idle,
            PlayState::Stopped => ZoneStatus::Stopped,
        }
    };
    let duration_ms = now_playing
        .as_ref()
        .map(|now| now.duration_ms)
        .filter(|duration| *duration > 0)
        .unwrap_or(renderer_state.duration_ms);
    let fixed = effective.volume_mode == VolumeMode::Fixed;
    let mut volume = ZoneVolume::new(
        if fixed { 100 } else { renderer_state.volume },
        renderer_state.muted,
        settings.limits(),
    );
    if effective.volume_mode == VolumeMode::Hardware
        && let Some(mut range) = info.volume_range.clone()
    {
        // Until the device reports a reading, show where the level maps to.
        range.current = renderer_state.device_volume.or_else(|| {
            Some(crate::outputs::renderer::to_device_scale(
                renderer_state.volume,
                &range,
            ))
        });
        volume.device_range = Some(range);
    }
    let is_device = info.kind == ZoneKind::Device;
    let stream = if info.network {
        current_song.and_then(|song_id| {
            network_stream_info(
                cache?,
                song_id,
                &settings,
                effective.limits,
                &info.name,
                queue?,
            )
        })
    } else {
        renderer_state.stream.clone()
    };
    let parson = source == ZoneSource::Parson;
    let mut setting_notes = vec![if info.network {
        "Leveling and DSP are applied to the audio Parson serves, from the next track. Either one means the renderer no longer gets the original file.".to_string()
    } else {
        "Leveling, DSP and software volume change the samples, so the zone is no longer bit-perfect while they are on.".to_string()
    }];
    if is_device {
        setting_notes.push(
            "Exclusive mode opens the device directly at each file's own sample rate and bit depth. Other apps cannot play on it while Parson does; Parson lets go when the queue ends."
                .to_string(),
        );
    }
    let mut view_settings = settings_view(
        &settings,
        &info.name,
        ZoneSettingsSupport {
            crossfade: false,
            leveling: true,
            volume_limits: !fixed,
            private: is_device,
            dsp: true,
            exclusive: info.exclusive_capable,
            volume_modes: info.volume_modes.clone(),
            max_format: true,
        },
        setting_notes,
    );
    view_settings.volume_mode = effective.volume_mode;
    let note = if is_device {
        // Devices decode the file themselves: nothing is re-encoded.
        format!(
            "This device plays up to {}/{}-bit. Files within that reach it untouched (bit-perfect in exclusive mode); anything else is resampled to the nearest rate it takes and fitted to its bit depth.",
            format_rate(effective.limits.max_sample_rate),
            effective.limits.max_bit_depth
        )
    } else {
        effective.limits.note("This renderer")
    };
    let mut notes = info.notes.clone();
    notes.insert(0, note.clone());
    if fixed {
        notes.push(
            "Fixed volume: Parson sends full-scale audio; set the level on your amplifier."
                .to_string(),
        );
    }
    if source == ZoneSource::External {
        notes.push("Another app is playing here. Playing from Parson replaces it.".to_string());
    }
    Zone {
        id: id.clone(),
        kind: info.kind,
        ecosystem: info.kind.ecosystem(),
        name: settings.name.clone().unwrap_or_else(|| info.name.clone()),
        icon: zone_icon(&settings, info.kind),
        device: info.device.clone(),
        status,
        source,
        now_playing,
        queue: ZoneQueue {
            song_ids: queue
                .map(|queue| queue.song_ids.clone())
                .unwrap_or_default(),
            current_position: current,
        },
        repeat: queue.map(|queue| queue.repeat).unwrap_or_default(),
        shuffle: queue.is_some_and(|queue| queue.shuffle),
        position_ms: extrapolate_position(
            renderer_state.position_ms,
            renderer_state.polled_at_ms,
            now,
            renderer_state.state == PlayState::Playing,
            duration_ms,
        ),
        duration_ms,
        volume: Some(volume),
        group: None,
        grouped_into: None,
        capabilities: ZoneCapabilities {
            transport: source != ZoneSource::None,
            seek: parson,
            volume: !fixed,
            mute: true,
            queue: parson,
            queue_edit: parson,
            repeat: parson,
            shuffle: parson,
            group: !settings.private,
            transfer_target: renderer_state.reachable,
            sleep_timer: true,
        },
        sleep_timer: sleep_view(state, &id),
        settings: view_settings,
        output: Some(ZoneOutputInfo {
            max_sample_rate: effective.limits.max_sample_rate,
            max_bit_depth: effective.limits.max_bit_depth,
            formats: effective.limits.formats(),
            note,
            exclusive: is_device.then_some(settings.exclusive),
            volume_mode: Some(effective.volume_mode),
            stream,
        }),
        notes,
    }
}

/// Signal path to a network renderer: what the media endpoint will serve.
fn network_stream_info(
    cache: &LibraryCache,
    song_id: &str,
    settings: &ZoneSettings,
    limits: FormatLimits,
    renderer: &str,
    queue: &HostedQueue,
) -> Option<ZoneStreamInfo> {
    let song = cache.song(song_id)?;
    let source = source_format(song);
    let leveling = sonos_leveling(settings, &queue.song_ids, queue.current, cache);
    let processed = leveling.is_some() || settings.dsp.is_active();
    let plan = crate::api::library::renderer_stream_plan(song, limits, processed);
    let mut steps = Vec::new();
    let output = match (plan.convert_to, plan.lossy) {
        (_, true) => {
            steps.push(
                "Converted to 320 kbps MP3 (the renderer takes no lossless format)".to_string(),
            );
            format!("MP3 320 kbps to {renderer}")
        }
        (None, false) => format!("Original file to {renderer}"),
        (Some((rate, depth)), false) => {
            if !processed
                || source.sample_rate != Some(rate)
                || source.bit_depth.unwrap_or(16) > depth
            {
                steps.push(format!(
                    "Converted to FLAC {}/{depth}-bit (the renderer's limit)",
                    format_rate(rate)
                ));
            }
            format!("FLAC {}/{depth}-bit to {renderer}", format_rate(rate))
        }
    };
    if leveling.is_some() {
        steps.push("Volume leveling".to_string());
    }
    if settings.dsp.is_active() {
        steps.push(format!("DSP: {}", settings.dsp.summary()));
    }
    Some(ZoneStreamInfo {
        source: describe_source(&source),
        output,
        bit_perfect: plan.convert_to.is_none() && !plan.lossy && source.lossless(),
        steps,
    })
}
