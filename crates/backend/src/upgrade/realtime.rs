//! "AI enhance in real time (experimental)": the track that plays on a
//! server output is streamed through the enhancer worker ahead of the
//! playhead, in overlapping chunks that are crossfaded together.
//!
//! The original decode is never held back by the worker. A reader thread
//! decodes ahead (up to the lookahead) and an enhancer thread sends the
//! next chunk that has not played yet. When a chunk's turn comes and its
//! enhanced copy is not back, the original chunk plays instead, crossfaded
//! like any other boundary; after a few misses in a row the stream stops
//! asking the worker and plays the original to the end. Nothing waits
//! except, briefly, the very first chunk.
//!
//! Chunk `k` covers frames `[kH, kH + H + O)` of the track (hop `H`,
//! overlap `O`). It emits `[kH, kH + H)`: its first `O` frames crossfaded
//! with the previous chunk's last `O`, then its own frames up to `kH + H`,
//! keeping its last `O` frames for the next crossfade.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Enhances blocks of interleaved `f32` samples. A session lives on the
/// enhancer thread (so an HTTP client's runtime is made and dropped there).
pub trait ChunkEnhancer: Send + Sync {
    fn session(&self) -> Box<dyn EnhanceSession>;
}

pub trait EnhanceSession {
    fn enhance(
        &mut self,
        samples: &[f32],
        channels: u8,
        sample_rate: u32,
        timeout: Duration,
    ) -> Result<Vec<f32>, String>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealtimeConfig {
    /// Hop between chunks, in frames.
    pub hop_frames: usize,
    /// Crossfade length, in frames.
    pub overlap_frames: usize,
    /// Chunks decoded ahead of the playhead.
    pub lookahead_chunks: usize,
    /// How long the first chunk may wait for its enhanced copy.
    pub first_chunk_wait: Duration,
    /// Misses in a row before the stream stops asking the worker.
    pub max_misses: usize,
}

impl RealtimeConfig {
    /// 4 s chunks with 250 ms crossfades and 12 s of lookahead.
    pub fn standard(sample_rate: u32) -> Self {
        let rate = sample_rate as usize;
        Self {
            hop_frames: rate * 4,
            overlap_frames: rate / 4,
            lookahead_chunks: 3,
            first_chunk_wait: Duration::from_millis(2_500),
            max_misses: 3,
        }
    }

    pub fn chunk_seconds(&self, sample_rate: u32) -> f64 {
        self.hop_frames as f64 / f64::from(sample_rate.max(1))
    }
}

/// Live counters, for the DSP panel.
#[derive(Default, Debug)]
pub struct RealtimeStats {
    pub enhanced_chunks: AtomicU64,
    pub original_chunks: AtomicU64,
    pub gave_up: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RealtimeStatus {
    pub enhanced_chunks: u64,
    pub original_chunks: u64,
    pub gave_up: bool,
}

impl RealtimeStats {
    pub fn snapshot(&self) -> RealtimeStatus {
        RealtimeStatus {
            enhanced_chunks: self.enhanced_chunks.load(Ordering::Relaxed),
            original_chunks: self.original_chunks.load(Ordering::Relaxed),
            gave_up: self.gave_up.load(Ordering::Relaxed),
        }
    }
}

/// The enhancer as the output pipeline sees it.
pub struct RealtimeBackend {
    pub enhancer: Arc<dyn ChunkEnhancer>,
    /// e.g. `Apollo`.
    pub model_label: String,
    /// The worker's measured real-time factor.
    pub realtime_factor: f64,
    /// The rate the model works at (44.1 kHz for Apollo).
    pub sample_rate: u32,
    /// Counters across every stream since the backend was set.
    pub stats: Arc<RealtimeStats>,
}

impl std::fmt::Debug for RealtimeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealtimeBackend")
            .field("model_label", &self.model_label)
            .field("realtime_factor", &self.realtime_factor)
            .finish()
    }
}

static BACKEND: OnceLock<RwLock<Option<Arc<RealtimeBackend>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<RealtimeBackend>>> {
    BACKEND.get_or_init(|| RwLock::new(None))
}

/// The worker real-time enhancement can use now, if any.
pub fn backend() -> Option<Arc<RealtimeBackend>> {
    slot().read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Set when the worker is configured and fast enough; cleared otherwise.
pub fn set_backend(backend: Option<RealtimeBackend>) {
    *slot().write().unwrap_or_else(|p| p.into_inner()) = backend.map(Arc::new);
}

/* ------------------------------------------------------------------ */
/* Overlap-add                                                         */
/* ------------------------------------------------------------------ */

/// Joins chunks: emits each chunk's first `overlap` frames crossfaded
/// with the previous chunk's held tail, then its middle, and holds its
/// own tail. Pure, so the boundary maths is tested without threads.
#[derive(Debug)]
pub struct OverlapJoiner {
    channels: usize,
    hop: usize,
    overlap: usize,
    held: Vec<f64>,
}

impl OverlapJoiner {
    pub fn new(channels: usize, hop: usize, overlap: usize) -> Self {
        assert!(overlap <= hop, "the overlap fits in a hop");
        Self {
            channels,
            hop,
            overlap,
            held: Vec::new(),
        }
    }

    /// Adds chunk `samples` (interleaved, up to `hop + overlap` frames)
    /// and returns the frames that are now final.
    pub fn push(&mut self, samples: &[f64]) -> Vec<f64> {
        let channels = self.channels;
        let frames = samples.len() / channels;
        let fade = self.overlap.min(frames).min(self.held.len() / channels);
        let mut out = Vec::with_capacity(self.hop * channels);
        for frame in 0..fade {
            // Linear crossfade: both sides are the same music, so their
            // sum keeps the level (equal-gain for correlated signals).
            let weight = (frame as f64 + 0.5) / fade as f64;
            for channel in 0..channels {
                let old = self.held[frame * channels + channel];
                let new = samples[frame * channels + channel];
                out.push(old * (1.0 - weight) + new * weight);
            }
        }
        let emit_until = self.hop.min(frames);
        out.extend_from_slice(&samples[fade * channels..emit_until * channels]);
        self.held = samples[emit_until * channels..].to_vec();
        out
    }

    /// The last chunk's tail, at the end of the track.
    pub fn finish(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.held)
    }
}

/* ------------------------------------------------------------------ */
/* The stream                                                          */
/* ------------------------------------------------------------------ */

struct Slot {
    index: usize,
    original: Arc<Vec<f64>>,
    enhanced: Option<Vec<f64>>,
    /// The enhancer is working on it.
    in_flight: bool,
}

#[derive(Default)]
struct Shared {
    slots: VecDeque<Slot>,
    /// Index of the next chunk the consumer takes.
    next_to_play: usize,
    eof: bool,
    read_error: Option<String>,
    shutdown: bool,
    gave_up: bool,
    misses: usize,
}

struct State {
    shared: Mutex<Shared>,
    changed: Condvar,
}

impl State {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A `PcmSource` of interleaved `f64le` frames, enhanced where the worker
/// keeps up and original where it does not.
pub struct AiEnhancedPcm {
    state: Arc<State>,
    joiner: OverlapJoiner,
    config: RealtimeConfig,
    stats: Arc<RealtimeStats>,
    pending: Vec<u8>,
    offset: usize,
    finished: bool,
    first: bool,
}

fn read_frames(source: &mut dyn Read, frames: usize, channels: usize) -> io::Result<Vec<f64>> {
    let mut bytes = vec![0u8; frames * channels * 8];
    let mut filled = 0;
    while filled < bytes.len() {
        let count = source.read(&mut bytes[filled..])?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    let whole = filled / (channels * 8) * channels * 8;
    Ok(bytes[..whole]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect())
}

impl AiEnhancedPcm {
    /// Wraps `original`, interleaved `f64le` at `sample_rate`.
    pub fn new(
        mut original: Box<dyn Read + Send>,
        channels: u8,
        sample_rate: u32,
        enhancer: Arc<dyn ChunkEnhancer>,
        config: RealtimeConfig,
        stats: Arc<RealtimeStats>,
    ) -> Self {
        stats.gave_up.store(false, Ordering::Relaxed);
        let state = Arc::new(State {
            shared: Mutex::new(Shared::default()),
            changed: Condvar::new(),
        });
        let channel_count = usize::from(channels.max(1));
        // Reader: decodes ahead, never waiting for the worker.
        {
            let state = state.clone();
            std::thread::Builder::new()
                .name("ai-enhance-reader".into())
                .spawn(move || {
                    let mut tail: Vec<f64> = Vec::new();
                    let mut index = 0usize;
                    loop {
                        {
                            let mut shared = state.lock();
                            while !shared.shutdown
                                && index >= shared.next_to_play + config.lookahead_chunks
                            {
                                shared = state
                                    .changed
                                    .wait(shared)
                                    .unwrap_or_else(|p| p.into_inner());
                            }
                            if shared.shutdown {
                                return;
                            }
                        }
                        let wanted = if index == 0 {
                            config.hop_frames + config.overlap_frames
                        } else {
                            config.hop_frames
                        };
                        let fresh = match read_frames(original.as_mut(), wanted, channel_count) {
                            Ok(fresh) => fresh,
                            Err(error) => {
                                let mut shared = state.lock();
                                shared.read_error = Some(error.to_string());
                                shared.eof = true;
                                state.changed.notify_all();
                                return;
                            }
                        };
                        let short = fresh.len() < wanted * channel_count;
                        let mut window = std::mem::take(&mut tail);
                        window.extend_from_slice(&fresh);
                        let mut shared = state.lock();
                        if window.is_empty() {
                            shared.eof = true;
                            state.changed.notify_all();
                            return;
                        }
                        // The next chunk starts `hop` frames in.
                        let hop = config.hop_frames * channel_count;
                        if window.len() > hop {
                            tail = window[hop..].to_vec();
                        }
                        shared.slots.push_back(Slot {
                            index,
                            original: Arc::new(window),
                            enhanced: None,
                            in_flight: false,
                        });
                        index += 1;
                        if short {
                            shared.eof = true;
                            state.changed.notify_all();
                            return;
                        }
                        state.changed.notify_all();
                    }
                })
                .expect("ai-enhance reader thread");
        }
        // Enhancer: sends the earliest chunk that has not played yet.
        {
            let state = state.clone();
            let stats = stats.clone();
            std::thread::Builder::new()
                .name("ai-enhance-worker".into())
                .spawn(move || {
                    let mut session = enhancer.session();
                    let timeout = Duration::from_secs_f64(
                        (config.hop_frames as f64 / f64::from(sample_rate.max(1))) * 2.0,
                    )
                    .max(Duration::from_secs(1));
                    let mut errors = 0usize;
                    loop {
                        let job = {
                            let mut shared = state.lock();
                            loop {
                                if shared.shutdown || shared.gave_up {
                                    return;
                                }
                                let next_to_play = shared.next_to_play;
                                if let Some(slot) = shared.slots.iter_mut().find(|slot| {
                                    slot.index >= next_to_play
                                        && slot.enhanced.is_none()
                                        && !slot.in_flight
                                }) {
                                    slot.in_flight = true;
                                    break (slot.index, slot.original.clone());
                                }
                                if shared.eof
                                    && shared.slots.iter().all(|slot| slot.index < next_to_play)
                                {
                                    return;
                                }
                                shared = state
                                    .changed
                                    .wait(shared)
                                    .unwrap_or_else(|p| p.into_inner());
                            }
                        };
                        let (index, original) = job;
                        let block: Vec<f32> = original.iter().map(|value| *value as f32).collect();
                        let result = session.enhance(&block, channels, sample_rate, timeout);
                        let mut shared = state.lock();
                        let slot = shared.slots.iter_mut().find(|slot| slot.index == index);
                        match (result, slot) {
                            (Ok(enhanced), Some(slot)) if enhanced.len() == block.len() => {
                                errors = 0;
                                slot.enhanced =
                                    Some(enhanced.iter().map(|value| f64::from(*value)).collect());
                                slot.in_flight = false;
                            }
                            (result, slot) => {
                                // Attempted: `in_flight` stays set, so the
                                // chunk is not sent again; it plays original.
                                if let Some(slot) = slot {
                                    slot.in_flight = true;
                                }
                                if let Err(error) = result {
                                    errors += 1;
                                    tracing::debug!(%error, "real-time enhancement of a chunk failed");
                                    if errors >= 2 {
                                        tracing::warn!(
                                            "the enhancer keeps failing; playing the original"
                                        );
                                        shared.gave_up = true;
                                        stats.gave_up.store(true, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                        state.changed.notify_all();
                    }
                })
                .expect("ai-enhance worker thread");
        }
        Self {
            state,
            joiner: OverlapJoiner::new(channel_count, config.hop_frames, config.overlap_frames),
            config,
            stats,
            pending: Vec::new(),
            offset: 0,
            finished: false,
            first: true,
        }
    }

    /// The next chunk's samples (enhanced or original), or `None` at the
    /// end of the track.
    fn next_chunk(&mut self) -> io::Result<Option<Vec<f64>>> {
        let mut shared = self.state.lock();
        let index = shared.next_to_play;
        // Wait for the original (decoding is far faster than playback).
        loop {
            if shared.slots.iter().any(|slot| slot.index == index) {
                break;
            }
            if let Some(error) = shared.read_error.take() {
                return Err(io::Error::other(error));
            }
            if shared.eof {
                return Ok(None);
            }
            shared = self
                .state
                .changed
                .wait(shared)
                .unwrap_or_else(|p| p.into_inner());
        }
        // The first chunk may wait briefly for its enhanced copy.
        if self.first && !shared.gave_up {
            let deadline = Instant::now() + self.config.first_chunk_wait;
            loop {
                let ready = shared
                    .slots
                    .iter()
                    .find(|slot| slot.index == index)
                    .is_some_and(|slot| slot.enhanced.is_some());
                let now = Instant::now();
                if ready || shared.gave_up || now >= deadline {
                    break;
                }
                shared = self
                    .state
                    .changed
                    .wait_timeout(shared, deadline - now)
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
        }
        self.first = false;
        let position = shared
            .slots
            .iter()
            .position(|slot| slot.index == index)
            .expect("the slot was there");
        let slot = shared.slots.remove(position).expect("slot");
        // Older slots can go too.
        shared.slots.retain(|other| other.index > index);
        shared.next_to_play = index + 1;
        let samples = match slot.enhanced {
            Some(enhanced) => {
                shared.misses = 0;
                self.stats.enhanced_chunks.fetch_add(1, Ordering::Relaxed);
                enhanced
            }
            None => {
                if !shared.gave_up {
                    shared.misses += 1;
                    if shared.misses >= self.config.max_misses {
                        tracing::info!(
                            "the enhancer fell behind playback; playing the original from here"
                        );
                        shared.gave_up = true;
                        self.stats.gave_up.store(true, Ordering::Relaxed);
                    }
                }
                self.stats.original_chunks.fetch_add(1, Ordering::Relaxed);
                slot.original.as_ref().clone()
            }
        };
        self.state.changed.notify_all();
        Ok(Some(samples))
    }

    fn refill(&mut self) -> io::Result<()> {
        self.pending.clear();
        self.offset = 0;
        while self.pending.is_empty() && !self.finished {
            let frames = match self.next_chunk()? {
                Some(samples) => self.joiner.push(&samples),
                None => {
                    self.finished = true;
                    self.joiner.finish()
                }
            };
            self.pending
                .extend(frames.iter().flat_map(|value| value.to_le_bytes()));
        }
        Ok(())
    }
}

impl Read for AiEnhancedPcm {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.offset >= self.pending.len() {
            if self.finished {
                return Ok(0);
            }
            self.refill()?;
            if self.pending.is_empty() {
                return Ok(0);
            }
        }
        let count = buffer.len().min(self.pending.len() - self.offset);
        buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

impl Drop for AiEnhancedPcm {
    fn drop(&mut self) {
        self.state.lock().shutdown = true;
        self.state.changed.notify_all();
    }
}

/* ------------------------------------------------------------------ */
/* The HTTP worker                                                     */
/* ------------------------------------------------------------------ */

/// The worker reached over HTTP.
pub struct HttpChunkEnhancer {
    pub config: super::enhancer::EnhancerConfig,
}

struct HttpSession {
    runtime: Option<tokio::runtime::Runtime>,
    client: Option<super::enhancer::EnhancerClient>,
}

impl ChunkEnhancer for HttpChunkEnhancer {
    fn session(&self) -> Box<dyn EnhanceSession> {
        Box::new(HttpSession {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok(),
            client: super::enhancer::EnhancerClient::new(self.config.clone()).ok(),
        })
    }
}

impl EnhanceSession for HttpSession {
    fn enhance(
        &mut self,
        samples: &[f32],
        channels: u8,
        sample_rate: u32,
        timeout: Duration,
    ) -> Result<Vec<f32>, String> {
        let (Some(runtime), Some(client)) = (&self.runtime, &self.client) else {
            return Err("the enhancer client could not start".into());
        };
        runtime
            .block_on(client.realtime(samples, channels, sample_rate, timeout))
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(frames: usize, channels: usize) -> Vec<f64> {
        (0..frames * channels)
            .map(|i| (i / channels) as f64)
            .collect()
    }

    /// Splits `signal` into the windows the reader makes.
    fn windows(signal: &[f64], channels: usize, hop: usize, overlap: usize) -> Vec<Vec<f64>> {
        let frames = signal.len() / channels;
        let mut out = Vec::new();
        let mut start = 0;
        while start < frames {
            let end = (start + hop + overlap).min(frames);
            out.push(signal[start * channels..end * channels].to_vec());
            if end == frames {
                break;
            }
            start += hop;
        }
        out
    }

    #[test]
    fn overlap_add_of_identical_chunks_reconstructs_the_signal() {
        let channels = 2;
        let signal = ramp(1_000, channels);
        let mut joiner = OverlapJoiner::new(channels, 300, 50);
        let mut out = Vec::new();
        for window in windows(&signal, channels, 300, 50) {
            out.extend(joiner.push(&window));
        }
        out.extend(joiner.finish());
        assert_eq!(out.len(), signal.len());
        for (a, b) in out.iter().zip(&signal) {
            assert!((a - b).abs() < 1e-9);
        }
    }

    #[test]
    fn a_fallback_chunk_is_crossfaded_not_cut() {
        // Enhanced chunks are 0.5x; chunk 1 falls back to the original.
        let channels = 1;
        let signal = vec![1.0; 900];
        let mut joiner = OverlapJoiner::new(channels, 300, 100);
        let parts = windows(&signal, channels, 300, 100);
        let mut out = Vec::new();
        for (index, window) in parts.iter().enumerate() {
            let chunk: Vec<f64> = if index == 1 {
                window.clone()
            } else {
                window.iter().map(|v| v * 0.5).collect()
            };
            out.extend(joiner.push(&chunk));
        }
        out.extend(joiner.finish());
        assert_eq!(out.len(), 900);
        // No step bigger than one crossfade increment at the boundaries.
        let biggest_step = out
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f64::max);
        assert!(biggest_step < 0.011, "{biggest_step}");
        assert_eq!(out[450], 1.0, "the fallback chunk plays the original");
        assert!(
            (out[350] - 0.75).abs() < 0.01,
            "crossfaded in: {}",
            out[350]
        );
        assert_eq!(out[50], 0.5);
    }

    struct Fake {
        delay: Duration,
        calls: Arc<AtomicU64>,
        fail: bool,
    }

    struct FakeSession {
        delay: Duration,
        calls: Arc<AtomicU64>,
        fail: bool,
    }

    impl ChunkEnhancer for Fake {
        fn session(&self) -> Box<dyn EnhanceSession> {
            Box::new(FakeSession {
                delay: self.delay,
                calls: self.calls.clone(),
                fail: self.fail,
            })
        }
    }

    impl EnhanceSession for FakeSession {
        fn enhance(
            &mut self,
            samples: &[f32],
            _: u8,
            _: u32,
            _: Duration,
        ) -> Result<Vec<f32>, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(self.delay);
            if self.fail {
                return Err("worker down".into());
            }
            Ok(samples.iter().map(|s| s * 0.5).collect())
        }
    }

    fn source(frames: usize, channels: usize) -> Box<dyn Read + Send> {
        let bytes: Vec<u8> = (0..frames * channels)
            .flat_map(|_| 1.0f64.to_le_bytes())
            .collect();
        Box::new(std::io::Cursor::new(bytes))
    }

    fn config() -> RealtimeConfig {
        RealtimeConfig {
            hop_frames: 1_000,
            overlap_frames: 100,
            lookahead_chunks: 3,
            first_chunk_wait: Duration::from_millis(500),
            max_misses: 2,
        }
    }

    fn drain(mut pcm: AiEnhancedPcm) -> Vec<f64> {
        let mut bytes = Vec::new();
        pcm.read_to_end(&mut bytes).unwrap();
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| f64::from_le_bytes(*chunk))
            .collect()
    }

    #[test]
    fn a_fast_worker_enhances_every_chunk_with_the_same_length() {
        let stats = Arc::new(RealtimeStats::default());
        let calls = Arc::new(AtomicU64::new(0));
        let pcm = AiEnhancedPcm::new(
            source(5_500, 2),
            2,
            44_100,
            Arc::new(Fake {
                delay: Duration::ZERO,
                calls: calls.clone(),
                fail: false,
            }),
            RealtimeConfig {
                first_chunk_wait: Duration::from_secs(5),
                ..config()
            },
            stats.clone(),
        );
        // Let the worker run ahead before playback starts pulling.
        std::thread::sleep(Duration::from_millis(200));
        let out = drain(pcm);
        assert_eq!(out.len(), 5_500 * 2, "same length as the original");
        assert!(stats.snapshot().enhanced_chunks >= 1);
        assert_eq!(out[10], 0.5, "the first chunk is enhanced");
    }

    #[test]
    fn a_failing_worker_falls_back_to_the_original_instantly() {
        let stats = Arc::new(RealtimeStats::default());
        let started = Instant::now();
        let pcm = AiEnhancedPcm::new(
            source(8_000, 1),
            1,
            44_100,
            Arc::new(Fake {
                delay: Duration::ZERO,
                calls: Arc::new(AtomicU64::new(0)),
                fail: true,
            }),
            config(),
            stats.clone(),
        );
        let out = drain(pcm);
        assert_eq!(out.len(), 8_000);
        assert!(
            out.iter().all(|value| (*value - 1.0).abs() < 1e-9),
            "original audio"
        );
        assert!(stats.snapshot().gave_up);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[actix_web::test]
    async fn the_http_worker_enhances_and_a_slow_one_falls_back() {
        use crate::upgrade::enhancer::EnhancerConfig;
        use crate::upgrade::fake::FakeEnhancer;
        let fake = FakeEnhancer::start("shared", 0.2, Vec::new()).await;
        let enhancer: Arc<dyn ChunkEnhancer> = Arc::new(HttpChunkEnhancer {
            config: EnhancerConfig {
                base_url: fake.url(),
                token: Some("shared".into()),
            },
        });
        let fast = {
            let enhancer = enhancer.clone();
            tokio::task::spawn_blocking(move || {
                let stats = Arc::new(RealtimeStats::default());
                let pcm = AiEnhancedPcm::new(
                    source(3_000, 2),
                    2,
                    44_100,
                    enhancer,
                    RealtimeConfig {
                        first_chunk_wait: Duration::from_secs(5),
                        ..config()
                    },
                    stats.clone(),
                );
                (drain(pcm), stats.snapshot())
            })
            .await
            .unwrap()
        };
        assert_eq!(fast.0.len(), 6_000);
        assert_eq!(fast.0[0], 0.5, "the worker's output plays");
        assert!(fast.1.enhanced_chunks >= 1);
        assert!(fake.realtime_calls() >= 1);

        fake.set_slow(true);
        let slow = tokio::task::spawn_blocking(move || {
            let stats = Arc::new(RealtimeStats::default());
            let pcm = AiEnhancedPcm::new(
                source(6_000, 1),
                1,
                44_100,
                enhancer,
                RealtimeConfig {
                    first_chunk_wait: Duration::from_millis(50),
                    ..config()
                },
                stats.clone(),
            );
            (drain(pcm), stats.snapshot())
        })
        .await
        .unwrap();
        assert_eq!(slow.0.len(), 6_000);
        assert_eq!(slow.0[0], 1.0, "the original plays when the worker is late");
        assert!(slow.1.gave_up);
        fake.stop().await;
    }

    #[test]
    fn a_slow_worker_never_stalls_playback() {
        let stats = Arc::new(RealtimeStats::default());
        let started = Instant::now();
        let pcm = AiEnhancedPcm::new(
            source(10_000, 1),
            1,
            44_100,
            Arc::new(Fake {
                delay: Duration::from_millis(1_500),
                calls: Arc::new(AtomicU64::new(0)),
                fail: false,
            }),
            config(),
            stats.clone(),
        );
        let out = drain(pcm);
        assert_eq!(out.len(), 10_000);
        let status = stats.snapshot();
        assert!(status.original_chunks >= 2, "{status:?}");
        assert!(status.gave_up, "stops asking after misses in a row");
        // Only the first chunk's short wait, never the worker's 1.5 s per
        // chunk times ten chunks.
        assert!(started.elapsed() < Duration::from_millis(2_000));
    }
}
