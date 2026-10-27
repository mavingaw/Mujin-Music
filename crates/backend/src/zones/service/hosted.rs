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
    TrackRequest, codec_name, describe_source, format_rate,
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
    pub dsp: DspSettings,
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
        output: settings.output(),
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
            .map(|renderer| {
                let mut zone =
                    hosted_zone_view(state, &self.loudness, renderer.as_ref(), cache, now);
                let members = renderer.group_members();
                if !members.is_empty() {
                    zone.group = Some(ZoneGroupInfo {
                        id: zone.id.clone(),
                        synced: true,
                        sync_note: crate::outputs::bridge::GROUP_SYNC_NOTE,
                        members: members
                            .iter()
                            .filter_map(|member| outputs.renderer(member))
                            .map(|member| {
                                let view = hosted_zone_view(
                                    state,
                                    &self.loudness,
                                    member.as_ref(),
                                    None,
                                    now,
                                );
                                ZoneGroupMember {
                                    id: view.id,
                                    name: view.name,
                                    kind: view.kind,
                                    volume: view.volume,
                                }
                            })
                            .collect(),
                    });
                }
                zone
            })
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
            dsp: settings.dsp.clone(),
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
            audio: song.audio.clone(),
            uri: String::new(),
            metadata: String::new(),
            mime_type: String::new(),
            title: song.name.clone(),
            artist: song.artist.clone(),
            album: String::new(),
            art_uri: None,
            leveling: None,
            dsp: settings.dsp.clone(),
        };
        if !info.network {
            // The device decodes the file itself; leveling becomes a gain.
            if let Some(leveling) = leveling {
                request.leveling = self
                    .loudness
                    .gain_for(
                        cache,
                        song_id,
                        leveling.by,
                        leveling.target_lufs,
                        leveling.adjustment_db,
                    )
                    .await
                    .map(|gain| crate::outputs::pipeline::TrackLeveling {
                        gain_db: f64::from(gain),
                        source: format!(
                            "{} loudness, {} LUFS target",
                            leveling.by.as_str(),
                            leveling.target_lufs
                        ),
                    });
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
            leveling.is_some() || settings.dsp.processes(),
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
        request.album = album.map(|album| album.name.clone()).unwrap_or_default();
        request.art_uri = album_art_uri.clone();
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
        user_id: i32,
        zone_id: &str,
        command: ZoneCommand,
        cache: Option<&LibraryCache>,
    ) -> Result<(), ZoneError> {
        let renderer = self.renderer(zone_id).ok_or(ZoneError::NotFound)?;
        // Plays here count for the profile that started or steered them.
        self.note_listener(zone_id, user_id, &command);
        // A member's own volume in a synced group.
        if let ZoneCommand::SetVolume {
            member_id: Some(member),
            ..
        }
        | ZoneCommand::AdjustVolume {
            member_id: Some(member),
            ..
        }
        | ZoneCommand::SetMute {
            member_id: Some(member),
            ..
        } = &command
        {
            if !renderer.group_members().contains(member) {
                return Err(ZoneError::Invalid("member_not_in_group"));
            }
            let member = member.clone();
            let mut command = command.clone();
            strip_member(&mut command);
            return Box::pin(self.hosted_command(user_id, &member, command, cache)).await;
        }
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
        if matches!(
            command,
            ZoneCommand::Load { .. }
                | ZoneCommand::Play
                | ZoneCommand::PlayPause
                | ZoneCommand::Next
                | ZoneCommand::Previous
                | ZoneCommand::Jump { .. }
                | ZoneCommand::Wake
        ) && self.lock().standby.remove(zone_id)
        {
            // Playing wakes the zone; it resumes where standby left it.
            self.changed(None);
        }
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
                return Box::pin(self.hosted_command(user_id, zone_id, next, cache)).await;
            }
            ZoneCommand::Stop => {
                if let Some(queue) = self.lock().hosted.get_mut(zone_id) {
                    queue.desired = Desired::Stopped;
                    queue.resume_ms = 0;
                }
                renderer.stop().await.map_err(device)
            }
            ZoneCommand::Standby => {
                // Stopping releases the device (a DAC can sleep, another app
                // can use it); the queue and position stay for waking up.
                let position = renderer.state().position_ms;
                {
                    let mut state = self.lock();
                    if let Some(queue) = state.hosted.get_mut(zone_id) {
                        queue.desired = Desired::Stopped;
                        queue.resume_ms = position;
                    }
                    state.standby.insert(zone_id.to_string());
                }
                renderer.stop().await.map_err(device)
            }
            ZoneCommand::Wake => Ok(()),
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
            | ZoneCommand::AddToQueue { .. }
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
        // Plays here count for the profile listening in this zone.
        if self.zone_listener(zone_id).is_some() {
            let observed = {
                let zones = self.lock();
                zones.hosted.get(zone_id).and_then(|queue| {
                    (state.serial == Some(queue.current_serial))
                        .then(|| queue.song_ids.get(queue.current).cloned())
                        .flatten()
                        .map(|song| (queue.current_serial, song))
                })
            };
            if let Some((serial, song_id)) = observed {
                let cache = self.library_cache().await;
                self.track_zone_listen(
                    zone_id,
                    &renderer.info().name,
                    serial,
                    &song_id,
                    state.position_ms,
                    state.state == PlayState::Playing,
                    cache.as_ref(),
                );
            }
        }
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
    loudness: &LoudnessStore,
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
    // Parson decodes for these itself and opens the DAC exclusively.
    let is_device = matches!(info.kind, ZoneKind::Device | ZoneKind::Bridge);
    let stream = if info.network {
        current_song
            .and_then(|song_id| {
                network_stream_info(
                    cache?,
                    loudness,
                    song_id,
                    &settings,
                    effective.limits,
                    &info.name,
                    queue?,
                )
            })
            .map(|stream| {
                let sent = current_song
                    .and_then(|song_id| cache?.song(song_id))
                    .and_then(|song| {
                        served_format(
                            song,
                            effective.limits,
                            settings.leveling != LevelingMode::Off || settings.dsp.processes(),
                        )
                    });
                with_renderer_report(stream, renderer_state.decoding.as_ref(), sent)
            })
    } else {
        renderer_state.stream.clone()
    };
    let parson = source == ZoneSource::Parson;
    let mut setting_notes = vec![if info.network {
        "Leveling and DSP are applied to the audio Mujin Music serves, from the next track. Either one means the renderer no longer gets the original file.".to_string()
    } else {
        "Leveling, DSP and software volume change the samples, so the zone is no longer bit-perfect while they are on.".to_string()
    }];
    if is_device {
        setting_notes.push(
            "Exclusive mode opens the device directly at each file's own sample rate and bit depth. Other apps cannot play on it while Mujin Music does; Mujin Music lets go when the queue ends."
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
            private: info.kind == ZoneKind::Device,
            dsp: true,
            dsd: is_device,
            exclusive: info.exclusive_capable,
            volume_modes: info.volume_modes.clone(),
            max_format: true,
            volume_control: true,
        },
        setting_notes,
    );
    view_settings.volume_mode = effective.volume_mode;
    let note = if info.kind == ZoneKind::Airplay {
        "AirPlay carries 44.1 kHz/16-bit lossless (ALAC) by design. Mujin Music decodes each file itself; 16-bit CD-quality files reach the receiver bit for bit, anything else is converted to 44.1 kHz/16-bit first.".to_string()
    } else if is_device {
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
            "Fixed volume: Mujin Music sends full-scale audio; set the level on your amplifier."
                .to_string(),
        );
    }
    if source == ZoneSource::External {
        notes
            .push("Another app is playing here. Playing from Mujin Music replaces it.".to_string());
    }
    let mut endpoints = renderer.endpoints(&effective.output);
    if let [endpoint] = endpoints.as_mut_slice() {
        endpoint.volume = Some(volume.clone());
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
            standby: true,
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
        endpoints,
        notes,
    }
}

/// Signal path to a network renderer: what the media endpoint will serve.
fn network_stream_info(
    cache: &LibraryCache,
    loudness: &LoudnessStore,
    song_id: &str,
    settings: &ZoneSettings,
    limits: FormatLimits,
    renderer: &str,
    queue: &HostedQueue,
) -> Option<ZoneStreamInfo> {
    let song = cache.song(song_id)?;
    let leveling = sonos_leveling(settings, &queue.song_ids, queue.current, cache);
    Some(served_stream_info(
        song,
        limits,
        cached_leveling_gain(loudness, cache, song_id, leveling),
        &settings.dsp,
        renderer,
    ))
}

/// The rate and depth a renderer gets for `song`: the FLAC Parson makes,
/// or the file's own; `None` for MP3.
fn served_format(
    song: &crate::domain::Song,
    limits: FormatLimits,
    processed: bool,
) -> Option<(u32, Option<u8>)> {
    let plan = crate::api::library::renderer_stream_plan(song, limits, processed);
    if plan.lossy {
        return None;
    }
    match plan.convert_to {
        Some((rate, depth)) => Some((rate, Some(depth))),
        None => {
            let source = source_format(song);
            Some((source.sample_rate?, source.bit_depth))
        }
    }
}

/// Adds what the renderer itself says it decodes (OpenHome `Info.Details`)
/// to a served stream's path: confirmation when it matches what Parson
/// sends (`sent`, rate and depth), a conversion when it does not.
fn with_renderer_report(
    mut stream: ZoneStreamInfo,
    decoding: Option<&crate::outputs::renderer::RendererDecoding>,
    sent: Option<(u32, Option<u8>)>,
) -> ZoneStreamInfo {
    let Some(decoding) = decoding else {
        return stream;
    };
    let depth = if decoding.bit_depth > 0 {
        format!("/{}-bit", decoding.bit_depth)
    } else {
        String::new()
    };
    let decoded = format!(
        "{} {}{depth}",
        decoding.codec,
        format_rate(decoding.sample_rate)
    );
    let matches = sent.is_some_and(|(rate, bits)| {
        rate == decoding.sample_rate
            && (decoding.bit_depth == 0 || bits.is_none_or(|bits| bits == decoding.bit_depth))
    });
    if matches && decoding.lossless {
        stream.steps.push(format!(
            "Renderer reports: decoding {decoded}, lossless, exactly what Mujin Music sends"
        ));
    } else {
        stream.steps.push(format!(
            "Renderer reports: decoding {decoded}{}, not what Mujin Music sends (the renderer converts it)",
            if decoding.lossless { ", lossless" } else { "" }
        ));
        stream.bit_perfect = false;
        if matches!(
            stream.quality,
            StreamQuality::Lossless | StreamQuality::Enhanced
        ) {
            stream.quality = StreamQuality::High;
        }
    }
    stream
}

/// Leveling as a signal path shows it: `None` when off, `Some(None)` while
/// the loudness is still being measured, `Some(Some(gain))` once known.
/// Reads only measured values, so building a zone view never runs ffmpeg.
pub(super) fn cached_leveling_gain(
    loudness: &LoudnessStore,
    cache: &LibraryCache,
    song_id: &str,
    leveling: Option<StreamLeveling>,
) -> Option<Option<f32>> {
    let leveling = leveling?;
    let measured = cache.song(song_id).and_then(|song| {
        let track = loudness.cached(song_id, std::path::Path::new(&song.path));
        match leveling.by {
            LevelBy::Album => loudness.album_cached(cache, song_id).or(track),
            LevelBy::Track => track,
        }
    });
    Some(measured.map(|measured| {
        super::super::leveling::gain_db(measured, leveling.target_lufs, leveling.adjustment_db)
    }))
}

/// The path from file to a renderer that Parson streams to (Sonos, UPnP,
/// Chromecast), built from the same plan the media endpoint follows, in the
/// order the stages run: decode, leveling, DSP, sample rate, bit depth,
/// encode.
pub(super) fn served_stream_info(
    song: &crate::domain::Song,
    limits: FormatLimits,
    leveling_gain: Option<Option<f32>>,
    dsp: &DspSettings,
    target: &str,
) -> ZoneStreamInfo {
    let source = source_format(song);
    let lossless = source.lossless();
    let processed = leveling_gain.is_some() || dsp.processes();
    let plan = crate::api::library::renderer_stream_plan(song, limits, processed);
    let codec = codec_name(&source);
    let mut steps = Vec::new();
    let mut converted = plan.lossy;
    let decodes = plan.lossy || plan.convert_to.is_some();
    if decodes && !lossless {
        steps.push(match plan.convert_to {
            Some((_, depth)) => format!("Decoded {codec} to {depth}-bit PCM"),
            None => format!("Decoded {codec}"),
        });
    }
    if decodes {
        match leveling_gain {
            Some(Some(gain)) => steps.push(format!("Volume leveling {gain:+.1} dB")),
            Some(None) => steps.push(
                "Volume leveling (measuring loudness; the gain shows here once measured)"
                    .to_string(),
            ),
            None => {}
        }
        if dsp.processes() {
            steps.push(format!("DSP: {}", dsp_summary(dsp)));
        }
    }
    let output = if plan.lossy {
        steps.push("Encoded as 320 kbps MP3 (the renderer takes no lossless format)".to_string());
        format!("MP3 320 kbps to {target}")
    } else if let Some((rate, depth)) = plan.convert_to {
        if let Some(source_rate) = source.sample_rate
            && source_rate != rate
        {
            converted = true;
            steps.push(format!(
                "Sample rate conversion {} to {} (linear phase)",
                format_rate(source_rate),
                format_rate(rate)
            ));
        }
        if lossless && let Some(bits) = source.bit_depth {
            if bits > depth {
                converted = true;
                steps.push(format!(
                    "Bit depth {bits}-bit to {depth}-bit{}",
                    if depth <= 16 { " (TPDF dither)" } else { "" }
                ));
            } else if bits < depth {
                steps.push(format!(
                    "Carried at {depth}-bit (from {bits}-bit) so the processing keeps full resolution"
                ));
            }
        }
        use crate::api::library::RenderContainer;
        let container = match plan.container {
            RenderContainer::Flac => "FLAC",
            RenderContainer::Wav => "WAV",
            RenderContainer::Lpcm => "LPCM",
        };
        steps.push(match plan.container {
            RenderContainer::Flac if !lossless => {
                format!("Sent as lossless FLAC {}/{depth}-bit", format_rate(rate))
            }
            RenderContainer::Flac if source.codec.as_deref() == Some("flac") => {
                "Re-encoded as FLAC (lossless)".to_string()
            }
            RenderContainer::Flac => format!("Transcoded {codec} to FLAC (lossless)"),
            RenderContainer::Wav | RenderContainer::Lpcm => format!(
                "Sent as uncompressed PCM ({container}, {}/{depth}-bit): the renderer takes no FLAC",
                format_rate(rate)
            ),
        });
        format!("{container} {}/{depth}-bit to {target}", format_rate(rate))
    } else {
        format!("Original {codec} file, untouched, to {target}")
    };
    ZoneStreamInfo {
        source: describe_source(&source),
        output,
        bit_perfect: plan.convert_to.is_none() && !plan.lossy && lossless,
        steps,
        quality: StreamQuality::rate(lossless && !plan.lossy, converted, processed && decodes),
        signal_path: None,
    }
}

#[cfg(test)]
mod signal_path_tests {
    use super::*;
    use crate::domain::{AudioProperties, Song};

    fn song(path: &str, codec: &str, rate: u32, depth: Option<u8>) -> Song {
        Song {
            path: path.to_string(),
            audio: Some(AudioProperties {
                codec: Some(codec.to_string()),
                sample_rate: Some(rate),
                bit_depth: depth,
                ..AudioProperties::default()
            }),
            ..Song::default()
        }
    }

    #[test]
    fn leveling_gain_and_hi_res_conversion_show_in_order() {
        let path = served_stream_info(
            &song("a.m4a", "alac", 192_000, Some(32)),
            FormatLimits::SONOS,
            Some(Some(-6.4)),
            &DspSettings::default(),
            "Living Room + Kitchen (Sonos Five, Sonos One)",
        );
        assert_eq!(path.source, "ALAC 192 kHz/32-bit");
        assert_eq!(
            path.steps,
            [
                "Volume leveling -6.4 dB",
                "Sample rate conversion 192 kHz to 48 kHz (linear phase)",
                "Bit depth 32-bit to 24-bit",
                "Transcoded ALAC to FLAC (lossless)",
            ]
        );
        assert_eq!(
            path.output,
            "FLAC 48 kHz/24-bit to Living Room + Kitchen (Sonos Five, Sonos One)"
        );
        assert_eq!(path.quality, StreamQuality::High);
    }

    #[test]
    fn sixteen_bit_renderers_get_dither_and_mp3_only_ones_are_low_quality() {
        let limits = FormatLimits {
            max_sample_rate: 48_000,
            max_bit_depth: 16,
            ..FormatLimits::SONOS
        };
        let path = served_stream_info(
            &song("a.flac", "flac", 48_000, Some(24)),
            limits,
            None,
            &DspSettings::default(),
            "Den",
        );
        assert_eq!(
            path.steps,
            [
                "Bit depth 24-bit to 16-bit (TPDF dither)",
                "Re-encoded as FLAC (lossless)"
            ]
        );
        assert_eq!(path.quality, StreamQuality::High);

        let mp3_only = FormatLimits {
            flac: false,
            wav: false,
            alac: false,
            aac: false,
            ..FormatLimits::SONOS
        };
        let path = served_stream_info(
            &song("a.flac", "flac", 44_100, Some(16)),
            mp3_only,
            None,
            &DspSettings::default(),
            "Radio",
        );
        assert_eq!(path.output, "MP3 320 kbps to Radio");
        assert_eq!(path.quality, StreamQuality::Low);
        assert!(!path.bit_perfect);
    }

    #[test]
    fn quality_takes_the_worst_stage() {
        assert_eq!(
            StreamQuality::rate(true, false, false),
            StreamQuality::Lossless
        );
        assert_eq!(
            StreamQuality::rate(true, false, true),
            StreamQuality::Enhanced
        );
        assert_eq!(StreamQuality::rate(true, true, true), StreamQuality::High);
        assert_eq!(StreamQuality::rate(false, false, false), StreamQuality::Low);
    }
}
