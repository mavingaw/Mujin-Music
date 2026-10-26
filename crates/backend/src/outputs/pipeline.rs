//! The one path from a library file to an output's samples.
//!
//! Every output the server plays itself (a DAC on this machine, a DAC on a
//! Parson Bridge, an AirPlay receiver) goes through here: the track is
//! planned against the device's formats and the zone's device setup
//! ([`plan_output`], or [`plan_dsd`] for DSD), the zone's DSP chain is
//! folded in ([`plan_processing_with`]), and the result is a decoder that
//! produces exactly the samples the device is opened for, plus the signal
//! path that says what happened on the way.
//!
//! - Nothing to change: ffmpeg decodes at the file's own rate and depth
//!   and the samples reach the device untouched (bit-perfect).
//! - A conversion the device forces: ffmpeg resamples (SoX when built in)
//!   or reduces the depth with triangular dither.
//! - DSP or volume leveling: ffmpeg decodes to 64-bit float and the
//!   server's own chain ([`DspChain`]) runs headroom, EQ, crossfeed,
//!   convolution and resampling before dithering to the device's depth.
//! - DSD: read from the file and sent natively or as DoP, never decoded,
//!   unless the strategy (or processing) asks for PCM.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use parson_output::convolution::FilterSummary;
use parson_output::dsd::{DsdPacking, DsdPcm, DsdReader, DsdSource, plan_dsd};
use parson_output::dsp::{
    ConvolutionFilter, DspChain, DspPcm, DspSettings, DspStats, ProcessingPlan, ResamplerKind,
    plan_processing_with,
};
use parson_output::player::{InputFormat, PcmOpener, PcmSource};
use parson_output::{
    DepthChange, DeviceCaps, DeviceInfo, Driver, DsdMode, OutputPlan, OutputSettings, PlanError,
    SampleFormat, SourceFormat, StreamFormat, plan_output,
};
use serde::{Deserialize, Serialize};

use crate::domain::{AudioProperties, codec_is_lossless};
use crate::signal_path::{
    NativeLeveling, NativeOutputTarget, ServerSignalPath, describe_native_leg,
};

/// Largest convolution filter file accepted.
pub const MAX_FILTER_BYTES: usize = 32 * 1024 * 1024;
/// Most filters kept on the server.
pub const MAX_FILTERS: usize = 32;

/// How decoded PCM is produced for a track; ffmpeg in production.
pub type DecoderFactory = Arc<
    dyn Fn(&Path, &OutputPlan, &SourceFormat, Option<f64>, u64) -> io::Result<PcmSource>
        + Send
        + Sync,
>;

/// What a float decoder produces: interleaved `f64le` at `sample_rate`
/// with `channels` channels. `resample_to` asks the decoder itself to
/// convert (for ratios the chain's polyphase table cannot hold).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatDecode {
    pub channels: u8,
    pub sample_rate: u32,
    pub resample_to: Option<u32>,
}

/// Decodes a track to 64-bit float for the DSP chain; ffmpeg in
/// production.
pub type FloatDecoderFactory =
    Arc<dyn Fn(&Path, &FloatDecode, u64) -> io::Result<PcmSource> + Send + Sync>;

/// Volume leveling the server applies to one track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackLeveling {
    pub gain_db: f64,
    /// Where the gain comes from, e.g. `track loudness` or `album
    /// ReplayGain`.
    pub source: String,
}

/// Why a track cannot be planned for a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanFailure {
    /// The track has no known sample rate (the library needs a rescan).
    UnknownFormat,
    Plan(PlanError),
}

impl std::fmt::Display for PlanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFormat => f.write_str(
                "Parson does not know this track's sample rate yet. Rescan the library, then try again.",
            ),
            Self::Plan(error) => write!(f, "This track cannot be played on the device: {error}."),
        }
    }
}

/// One track as the library knows it.
#[derive(Clone, Copy, Debug)]
pub struct TrackSource<'a> {
    pub song_id: &'a str,
    pub path: &'a str,
    pub audio: Option<&'a AudioProperties>,
}

/// Where the device is, for the signal path.
#[derive(Clone, Copy, Debug)]
pub struct DevicePlace<'a> {
    /// The machine the device is attached to (this server, or a bridge).
    pub host: &'a str,
}

/// What a track's samples become on the way to an output.
#[derive(Clone, Debug)]
pub struct SamplePlan {
    pub plan: OutputPlan,
    /// The server's DSP chain for the track, when anything processes.
    pub processing: Option<ProcessingPlan>,
    pub source: SourceFormat,
}

impl SamplePlan {
    /// How DSD is packed for the device, when it is sent as DSD.
    pub fn dsd_packing(&self) -> Option<DsdPacking> {
        self.plan.dsd.as_ref().and_then(|dsd| match dsd.mode {
            DsdMode::Native => Some(DsdPacking::Native(self.plan.sample_format)),
            DsdMode::Dop => Some(DsdPacking::Dop(self.plan.sample_format)),
            DsdMode::Pcm => None,
        })
    }

    /// What the output is opened with.
    pub fn stream(&self) -> StreamFormat {
        StreamFormat {
            sample_rate: self.plan.sample_rate,
            sample_format: self.plan.sample_format,
            channels: self.plan.channels,
        }
    }

    /// What the decoder writes.
    pub fn input(&self) -> InputFormat {
        if self.dsd_packing().is_some() {
            InputFormat::Raw
        } else {
            InputFormat::for_container(self.plan.sample_format)
        }
    }
}

/// How one track plays on one device.
#[derive(Clone, Debug)]
pub struct PlannedTrack {
    pub samples: SamplePlan,
    pub signal_path: ServerSignalPath,
    /// What the device is opened with.
    pub stream: StreamFormat,
    /// What the decoder writes.
    pub input: InputFormat,
    pub exclusive: bool,
}

/// Decoders and filters the server plays with.
pub struct Pipeline {
    decoder: DecoderFactory,
    float_decoder: FloatDecoderFactory,
    resampler: &'static str,
    pub filters: Arc<FilterStore>,
}

/// The process-wide pipeline: ffmpeg, and the filters in the data folder.
pub fn global() -> Arc<Pipeline> {
    static PIPELINE: OnceLock<Arc<Pipeline>> = OnceLock::new();
    Arc::clone(PIPELINE.get_or_init(|| {
        Arc::new(Pipeline::ffmpeg(Arc::new(FilterStore::new(Some(
            crate::settings::data_path(&[FILTER_DIR]),
        )))))
    }))
}

impl Pipeline {
    /// Decodes with ffmpeg, convolving with the filters in `filters`.
    pub fn ffmpeg(filters: Arc<FilterStore>) -> Self {
        Self {
            decoder: Arc::new(ffmpeg_decoder),
            float_decoder: Arc::new(ffmpeg_float_decoder),
            resampler: if ffmpeg_has_soxr() {
                "soxr, very high quality"
            } else {
                "ffmpeg resampler, 64-tap sinc"
            },
            filters,
        }
    }

    /// A pipeline with its own decoders (tests).
    pub fn with_decoders(
        decoder: DecoderFactory,
        float_decoder: FloatDecoderFactory,
        resampler: &'static str,
        filters: Arc<FilterStore>,
    ) -> Self {
        Self {
            decoder,
            float_decoder,
            resampler,
            filters,
        }
    }

    /// The same pipeline with another float decoder (tests).
    pub fn with_float_decoder(&self, float_decoder: FloatDecoderFactory) -> Self {
        Self {
            decoder: Arc::clone(&self.decoder),
            float_decoder,
            resampler: self.resampler,
            filters: Arc::clone(&self.filters),
        }
    }

    pub fn resampler(&self) -> &'static str {
        self.resampler
    }

    /// Plans what `audio` becomes on an output with `caps`: the format
    /// it is sent in and the DSP chain that runs on the way.
    pub fn plan_samples(
        &self,
        caps: Option<&DeviceCaps>,
        settings: &OutputSettings,
        dsp: &DspSettings,
        audio: Option<&AudioProperties>,
        leveling: Option<&TrackLeveling>,
    ) -> Result<SamplePlan, PlanFailure> {
        let (mut plan, source) = match dsd_source(audio) {
            Some(dsd_source) => {
                // DSP and leveling run on PCM, so they convert DSD first.
                let force_pcm = (dsp.processes() || leveling.is_some())
                    .then_some("the DSP engine or volume leveling is on, and both work on PCM");
                let plan =
                    plan_dsd(dsd_source, caps, settings, force_pcm).map_err(PlanFailure::Plan)?;
                (plan, parson_output::dsd::pcm_source(dsd_source))
            }
            None => {
                let source = source_format(audio).ok_or(PlanFailure::UnknownFormat)?;
                (
                    plan_output(source, caps, settings).map_err(PlanFailure::Plan)?,
                    source,
                )
            }
        };
        let sent_as_dsd = plan
            .dsd
            .as_ref()
            .is_some_and(|dsd| dsd.mode != DsdMode::Pcm);
        // Processing the listener asked for changes the samples on purpose;
        // it runs in the server's own chain, before the device.
        let filter = dsp
            .convolution_filter()
            .and_then(|id| self.filters.load(id));
        let processing = caps.filter(|_| !sent_as_dsd).and_then(|caps| {
            plan_processing_with(
                &mut plan,
                &source,
                dsp,
                leveling.map(|leveling| leveling.gain_db),
                caps,
                settings,
                filter.as_ref(),
            )
        });
        Ok(SamplePlan {
            plan,
            processing,
            source,
        })
    }

    /// Plans `track` on `device` with the zone's device setup and DSP.
    pub fn plan(
        &self,
        device: &DeviceInfo,
        settings: &OutputSettings,
        dsp: &DspSettings,
        track: TrackSource<'_>,
        leveling: Option<&TrackLeveling>,
        place: DevicePlace<'_>,
    ) -> Result<PlannedTrack, PlanFailure> {
        let mut settings = *settings;
        if device.driver.is_network() {
            // A network stream is never mixed, and DSD reaches network
            // players as PCM (AirPlay cannot carry it).
            settings.exclusive = true;
            settings.dsd_strategy = parson_output::DsdStrategy::Pcm;
        }
        let samples =
            self.plan_samples(device.caps.as_ref(), &settings, dsp, track.audio, leveling)?;
        let extension = Path::new(track.path)
            .extension()
            .and_then(|extension| extension.to_str());
        let mode_label = device.mode_label(settings.exclusive);
        let signal_path = describe_native_leg(
            track.song_id,
            track.audio,
            extension,
            &samples.plan,
            &NativeOutputTarget {
                device_name: &device.name,
                mode_label: &mode_label,
                host: place.host,
                config_pcm: device.is_alsa_config_pcm(),
                usb_id: device
                    .hardware
                    .as_ref()
                    .and_then(|hardware| hardware.usb_id.as_deref()),
                usb_speed: device
                    .hardware
                    .as_ref()
                    .and_then(|hardware| hardware.speed.as_deref()),
                simulated: device.driver == Driver::Simulated,
                kernel_emulator: device
                    .hardware
                    .as_ref()
                    .is_some_and(|hardware| hardware.kernel_emulator),
                resampler: self.resampler,
                leveling: leveling.map(|leveling| NativeLeveling {
                    gain_db: leveling.gain_db,
                    source: &leveling.source,
                }),
                processing: samples.processing.as_ref(),
                network: device.network.as_ref(),
            },
        );
        Ok(PlannedTrack {
            stream: samples.stream(),
            input: samples.input(),
            exclusive: settings.exclusive,
            samples,
            signal_path,
        })
    }

    /// Opens `path` at a position as `planned` says: DSD read from the
    /// file, PCM decoded exactly, or decoded to float and run through the
    /// DSP chain (whose live measurements go to `stats`).
    pub fn opener(&self, path: PathBuf, planned: &SamplePlan, stats: Arc<DspStats>) -> PcmOpener {
        if let Some(packing) = planned.dsd_packing() {
            return Box::new(move |position| open_dsd(&path, packing, position));
        }
        let Some(processing) = planned.processing.clone() else {
            let decoder = Arc::clone(&self.decoder);
            let plan = planned.plan.clone();
            let source = planned.source;
            return Box::new(move |position| decoder(&path, &plan, &source, None, position));
        };
        let impulse = processing
            .convolution
            .as_ref()
            .and_then(|convolution| self.filters.load(&convolution.filter_id))
            .map(|filter| filter.impulse);
        let decoder = Arc::clone(&self.float_decoder);
        let resample_to = (processing.resampler == Some(ResamplerKind::Decoder))
            .then_some(processing.output_rate);
        let spec = FloatDecode {
            channels: planned.plan.channels,
            sample_rate: planned.source.sample_rate,
            resample_to,
        };
        let input_rate = resample_to.unwrap_or(planned.source.sample_rate);
        let output = InputFormat::for_container(planned.plan.sample_format);
        Box::new(move |position| {
            let decoded = decoder(&path, &spec, position)?;
            let mut chain = DspChain::new(&processing, input_rate, output, Arc::clone(&stats));
            if let Some(impulse) = &impulse {
                chain = chain.with_convolution(impulse).map_err(io::Error::other)?;
            }
            Ok(Box::new(DspPcm::new(decoded, chain)) as PcmSource)
        })
    }
}

/* ------------------------------------------------------------------ */
/* Convolution filters                                                 */
/* ------------------------------------------------------------------ */

/// Folder of the data directory that holds uploaded filters.
pub const FILTER_DIR: &str = "convolution";

/// A convolution filter stored on the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FilterRecord {
    pub id: String,
    pub name: String,
    /// The uploaded file's name.
    pub file_name: Option<String>,
    pub added_at: String,
}

/// A filter as the API lists it.
#[derive(Serialize, Clone, Debug)]
pub struct FilterView {
    #[serde(flatten)]
    pub record: FilterRecord,
    pub summary: FilterSummary,
    /// Magnitude response of the first channel, `[Hz, dB]`, 20 Hz–20 kHz.
    pub response: Vec<[f64; 2]>,
}

/// Uploaded FIR filters (room correction and the like), one WAV file each
/// plus an index, in one folder of the data directory.
pub struct FilterStore {
    dir: Option<PathBuf>,
    cache: Mutex<HashMap<String, ConvolutionFilter>>,
}

/// A file of the filter folder: the index, or one filter's WAV. Only
/// these names are ever joined to the folder, so a request can never
/// reach outside it.
enum FilterFile<'a> {
    Index,
    Impulse(&'a str),
}

impl FilterStore {
    /// Filters in `dir`; `None` keeps none (nothing can be uploaded).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn file(&self, file: FilterFile<'_>) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        match file {
            FilterFile::Index => Some(dir.join("index.json")),
            FilterFile::Impulse(id) => {
                parson_output::dsp::valid_filter_id(id).then(|| dir.join(format!("{id}.wav")))
            }
        }
    }

    fn records(&self) -> Vec<FilterRecord> {
        self.file(FilterFile::Index)
            .and_then(|index| std::fs::read_to_string(index).ok())
            .and_then(|text| serde_json::from_str::<Vec<FilterRecord>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|record| parson_output::dsp::valid_filter_id(&record.id))
            .collect()
    }

    fn save_records(&self, records: &[FilterRecord]) -> io::Result<()> {
        let index = self
            .file(FilterFile::Index)
            .ok_or_else(|| io::Error::other("no filter folder"))?;
        let temp = index.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(records)?)?;
        std::fs::rename(temp, index)
    }

    fn view(record: FilterRecord, filter: &ConvolutionFilter) -> FilterView {
        FilterView {
            response: filter
                .impulse
                .response_db(0, 96)
                .into_iter()
                .map(|(frequency, db)| {
                    [
                        (frequency * 10.0).round() / 10.0,
                        (db * 100.0).round() / 100.0,
                    ]
                })
                .collect(),
            summary: filter.summary.clone(),
            record,
        }
    }

    /// The filters on the server, with their summaries.
    pub fn list(&self) -> Vec<FilterView> {
        self.records()
            .into_iter()
            .filter_map(|record| {
                let filter = self.load(&record.id)?;
                Some(Self::view(record, &filter))
            })
            .collect()
    }

    /// Reads a filter, keeping it in memory once read.
    pub fn load(&self, id: &str) -> Option<ConvolutionFilter> {
        if !parson_output::dsp::valid_filter_id(id) {
            return None;
        }
        if let Some(filter) = self.cache.lock().ok()?.get(id) {
            return Some(filter.clone());
        }
        let record = self.records().into_iter().find(|record| record.id == id)?;
        let bytes = std::fs::read(self.file(FilterFile::Impulse(id))?).ok()?;
        let (impulse, format) = parson_output::convolution::parse_wav(&bytes).ok()?;
        let filter = ConvolutionFilter {
            id: id.to_string(),
            name: record.name,
            summary: impulse.summary(format),
            impulse: Arc::new(impulse),
        };
        self.cache
            .lock()
            .ok()?
            .insert(id.to_string(), filter.clone());
        Some(filter)
    }

    /// The filter's WAV file, for a browser's own convolver.
    pub fn wav(&self, id: &str) -> Option<Vec<u8>> {
        if !self.records().iter().any(|record| record.id == id) {
            return None;
        }
        std::fs::read(self.file(FilterFile::Impulse(id))?).ok()
    }

    /// Stores an uploaded WAV impulse response.
    pub fn add(
        &self,
        name: &str,
        file_name: Option<&str>,
        bytes: &[u8],
    ) -> Result<FilterView, String> {
        if bytes.len() > MAX_FILTER_BYTES {
            return Err(format!(
                "A filter file can be at most {} MB.",
                MAX_FILTER_BYTES / (1024 * 1024)
            ));
        }
        let (impulse, format) = parson_output::convolution::parse_wav(bytes)?;
        let dir = self
            .dir
            .as_ref()
            .ok_or("This server has no data folder for filters.")?;
        let mut records = self.records();
        if records.len() >= MAX_FILTERS {
            return Err(format!(
                "Delete a filter first; the server keeps at most {MAX_FILTERS}."
            ));
        }
        let name: String = name
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        let name = if name.is_empty() {
            file_name
                .map(|file| {
                    file.trim_end_matches(".wav")
                        .trim_end_matches(".WAV")
                        .to_string()
                })
                .filter(|file| !file.is_empty())
                .unwrap_or_else(|| "Convolution filter".into())
        } else {
            name
        };
        let id = parson_output::network::random_hex(8);
        let wav = self
            .file(FilterFile::Impulse(&id))
            .ok_or("Could not name the filter.")?;
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("Could not save the filter: {error}"))?;
        std::fs::write(wav, bytes)
            .map_err(|error| format!("Could not save the filter: {error}"))?;
        let record = FilterRecord {
            id: id.clone(),
            name: name.clone(),
            file_name: file_name.map(|file| file.chars().take(120).collect()),
            added_at: chrono::Utc::now().to_rfc3339(),
        };
        records.push(record.clone());
        self.save_records(&records)
            .map_err(|error| format!("Could not save the filter list: {error}"))?;
        let filter = ConvolutionFilter {
            id: id.clone(),
            name,
            summary: impulse.summary(format),
            impulse: Arc::new(impulse),
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(id, filter.clone());
        }
        Ok(Self::view(record, &filter))
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        if self.dir.is_none() {
            return Err("This server has no data folder for filters.".into());
        }
        let mut records = self.records();
        let before = records.len();
        records.retain(|record| record.id != id);
        if records.len() == before {
            return Err("That filter is not on the server.".into());
        }
        self.save_records(&records)
            .map_err(|error| format!("Could not save the filter list: {error}"))?;
        if let Some(wav) = self.file(FilterFile::Impulse(id)) {
            let _ = std::fs::remove_file(wav);
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(id);
        }
        Ok(())
    }
}

/* ------------------------------------------------------------------ */
/* Sources and decoders                                                */
/* ------------------------------------------------------------------ */

/// This machine's host name, for the transport stage ("studio-pc ·
/// Parson audio transport"); `This server` when it cannot be read.
pub fn host_name() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is writable for its whole length, and
        // gethostname NUL-terminates within it on success.
        #[cfg(target_os = "linux")]
        let name = (unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0)
            .then(|| {
                let end = buffer
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(buffer.len());
                String::from_utf8_lossy(&buffer[..end]).trim().to_string()
            });
        #[cfg(not(target_os = "linux"))]
        let name = {
            let _ = &mut buffer;
            std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .ok()
        };
        name.filter(|name| !name.is_empty())
            .unwrap_or_else(|| "This server".to_string())
    })
}

/// A DSD track's rate and channels, from its indexed properties.
pub fn dsd_source(audio: Option<&AudioProperties>) -> Option<DsdSource> {
    let audio = audio?;
    if audio.codec.as_deref() != Some("dsd") {
        return None;
    }
    Some(DsdSource {
        rate: audio.sample_rate.filter(|rate| *rate > 0)?,
        channels: audio.channels.filter(|channels| *channels > 0).unwrap_or(2),
    })
}

/// Opens a DSD file at `position_ms`, packed for the device.
fn open_dsd(path: &Path, packing: DsdPacking, position_ms: u64) -> io::Result<PcmSource> {
    let mut file = io::BufReader::new(std::fs::File::open(path)?);
    let info = parson_output::dsd::read_info(&mut file)?;
    let per_frame = packing.bytes_per_frame() as u64;
    let start = info.byte_at_ms(position_ms) / per_frame * per_frame;
    Ok(Box::new(DsdPcm::new(
        DsdReader::new(file, info, start),
        info.channels,
        packing,
    )))
}

/// The decoder's view of a track; `None` without a known sample rate. A
/// track indexed before losslessness was recorded is judged by its codec.
pub fn source_format(audio: Option<&AudioProperties>) -> Option<SourceFormat> {
    let audio = audio?;
    let lossless = audio
        .lossless
        .or_else(|| audio.codec.as_deref().and_then(codec_is_lossless))
        == Some(true);
    Some(SourceFormat {
        sample_rate: audio.sample_rate.filter(|rate| *rate > 0)?,
        bit_depth: if lossless {
            audio.bit_depth.filter(|bits| (8..=32).contains(bits))
        } else {
            None
        },
        channels: audio.channels.filter(|channels| *channels > 0).unwrap_or(2),
        lossless,
    })
}

/// The `-af` filter for a plan, or `None` when the samples pass through
/// untouched (format widening is an exact shift ffmpeg does by itself).
pub fn decode_filter(plan: &OutputPlan, gain_db: Option<f64>, soxr: bool) -> Option<String> {
    let gain = gain_db.filter(|gain| gain.is_finite() && *gain != 0.0);
    let dither = plan.sample_format == SampleFormat::S16
        && (gain.is_some()
            || matches!(
                plan.depth,
                DepthChange::Reduce { .. } | DepthChange::Quantize { .. }
            ));
    if plan.resample.is_none() && !dither && gain.is_none() {
        return None;
    }
    let mut options = vec![format!("osr={}", plan.sample_rate)];
    if plan.resample.is_some() {
        if soxr {
            options.push("resampler=soxr".into());
            options.push("precision=28".into());
        } else {
            options.push("filter_size=64".into());
            options.push("phase_shift=10".into());
            options.push("cutoff=0.97".into());
        }
    }
    if dither || (plan.resample.is_some() && plan.sample_format == SampleFormat::S16) {
        options.push("dither_method=triangular".into());
    }
    let resample = format!("aresample={}", options.join(":"));
    Some(match gain {
        Some(gain) => format!("volume={gain:.2}dB:precision=double,{resample}"),
        None => resample,
    })
}

fn decode_prefix(path: &Path, start_ms: u64) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error"]
        .into_iter()
        .map(OsString::from)
        .collect();
    if start_ms > 0 {
        args.push("-ss".into());
        args.push(format!("{}.{:03}", start_ms / 1000, start_ms % 1000).into());
    }
    args.push("-i".into());
    args.push(path.as_os_str().to_owned());
    for arg in ["-map", "0:a:0", "-vn", "-sn", "-dn"] {
        args.push(arg.into());
    }
    args
}

/// ffmpeg arguments that decode `path` from `start_ms` to raw PCM for
/// `plan` on stdout.
pub fn decode_args(
    path: &Path,
    plan: &OutputPlan,
    gain_db: Option<f64>,
    start_ms: u64,
    soxr: bool,
) -> Vec<OsString> {
    let mut args = decode_prefix(path, start_ms);
    if let Some(filter) = decode_filter(plan, gain_db, soxr) {
        args.push("-af".into());
        args.push(filter.into());
    }
    let (codec, format) = match InputFormat::for_container(plan.sample_format) {
        InputFormat::S16 => ("pcm_s16le", "s16le"),
        // DSD is read by the server itself, never through ffmpeg.
        InputFormat::S32 | InputFormat::Raw => ("pcm_s32le", "s32le"),
    };
    for arg in [
        "-ac".to_string(),
        plan.channels.to_string(),
        "-ar".to_string(),
        plan.sample_rate.to_string(),
        "-c:a".to_string(),
        codec.to_string(),
        "-f".to_string(),
        format.to_string(),
        "pipe:1".to_string(),
    ] {
        args.push(arg.into());
    }
    args
}

/// ffmpeg arguments that decode `path` from `start_ms` to interleaved
/// 64-bit float for the DSP chain.
pub fn float_decode_args(
    path: &Path,
    spec: &FloatDecode,
    start_ms: u64,
    soxr: bool,
) -> Vec<OsString> {
    let mut args = decode_prefix(path, start_ms);
    let rate = spec.resample_to.unwrap_or(spec.sample_rate);
    if let Some(to) = spec.resample_to {
        args.push("-af".into());
        args.push(
            if soxr {
                format!("aresample=osr={to}:resampler=soxr:precision=28")
            } else {
                format!("aresample=osr={to}:filter_size=64:phase_shift=10:cutoff=0.97")
            }
            .into(),
        );
    }
    for arg in [
        "-ac".to_string(),
        spec.channels.to_string(),
        "-ar".to_string(),
        rate.to_string(),
        "-c:a".to_string(),
        "pcm_f64le".to_string(),
        "-f".to_string(),
        "f64le".to_string(),
        "pipe:1".to_string(),
    ] {
        args.push(arg.into());
    }
    args
}

/// Decodes `path` to 64-bit float with ffmpeg.
pub fn ffmpeg_float_decoder(
    path: &Path,
    spec: &FloatDecode,
    start_ms: u64,
) -> io::Result<PcmSource> {
    spawn_ffmpeg(float_decode_args(path, spec, start_ms, ffmpeg_has_soxr()))
}

fn spawn_ffmpeg(args: Vec<OsString>) -> io::Result<PcmSource> {
    let mut child = Command::new("ffmpeg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| io::Error::other(format!("could not start ffmpeg: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("ffmpeg has no output"))?;
    Ok(Box::new(ChildPcm { child, stdout }))
}

/// ffmpeg's stdout; the process is killed when this is dropped.
struct ChildPcm {
    child: Child,
    stdout: ChildStdout,
}

impl Read for ChildPcm {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stdout.read(buffer)
    }
}

impl Drop for ChildPcm {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decodes `path` for `plan` with ffmpeg.
pub fn ffmpeg_decoder(
    path: &Path,
    plan: &OutputPlan,
    _source: &SourceFormat,
    gain_db: Option<f64>,
    start_ms: u64,
) -> io::Result<PcmSource> {
    spawn_ffmpeg(decode_args(
        path,
        plan,
        gain_db,
        start_ms,
        ffmpeg_has_soxr(),
    ))
}

/// Whether ffmpeg was built with the SoX resampler.
pub fn ffmpeg_has_soxr() -> bool {
    static SOXR: OnceLock<bool> = OnceLock::new();
    *SOXR.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-buildconf"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout).contains("--enable-libsoxr")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal_path::SignalQuality;
    use parson_output::dsp::{CrossfeedStrength, EqBand, EqFilterType};
    use parson_output::simulated::SimulatedBackend;
    use parson_output::{Backend, DsdStrategy};

    fn audio(rate: u32, bits: u8, lossless: bool) -> AudioProperties {
        AudioProperties {
            codec: Some(if lossless { "flac" } else { "mp3" }.into()),
            container: Some(if lossless { "flac" } else { "mp3" }.into()),
            sample_rate: Some(rate),
            bit_depth: lossless.then_some(bits),
            channels: Some(2),
            bitrate_kbps: Some(if lossless { 2_000 } else { 320 }),
            lossless: Some(lossless),
        }
    }

    /// An endless 0.9 full-scale 1 kHz sine in `f64le`, standing in for
    /// ffmpeg's float decoder.
    struct SineDecoder {
        channels: u8,
        rate: u32,
        frame: u64,
        pending: Vec<u8>,
    }

    impl Read for SineDecoder {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.pending.is_empty() {
                for _ in 0..1_024 {
                    let t = self.frame as f64 / f64::from(self.rate);
                    let value = 0.9 * (2.0 * std::f64::consts::PI * 1_000.0 * t).sin();
                    for _ in 0..self.channels {
                        self.pending.extend_from_slice(&value.to_le_bytes());
                    }
                    self.frame += 1;
                }
            }
            let count = buffer.len().min(self.pending.len());
            buffer[..count].copy_from_slice(&self.pending[..count]);
            self.pending.drain(..count);
            Ok(count)
        }
    }

    fn pipeline(filters: Option<PathBuf>) -> Pipeline {
        Pipeline::with_decoders(
            Arc::new(|_, _, _, _, _| Ok(Box::new(io::repeat(0)) as PcmSource)),
            Arc::new(|_, spec, _| {
                Ok(Box::new(SineDecoder {
                    channels: spec.channels,
                    rate: spec.resample_to.unwrap_or(spec.sample_rate),
                    frame: 0,
                    pending: Vec::new(),
                }) as PcmSource)
            }),
            "test resampler",
            Arc::new(FilterStore::new(filters)),
        )
    }

    fn dac() -> DeviceInfo {
        SimulatedBackend::dac_up_to_192k().devices().remove(0)
    }

    fn plan(
        pipeline: &Pipeline,
        settings: &OutputSettings,
        dsp: &DspSettings,
        audio: Option<&AudioProperties>,
        leveling: Option<&TrackLeveling>,
    ) -> Result<PlannedTrack, PlanFailure> {
        pipeline.plan(
            &dac(),
            settings,
            dsp,
            TrackSource {
                song_id: "song-1",
                path: "/music/track.flac",
                audio,
            },
            leveling,
            DevicePlace { host: "studio" },
        )
    }

    fn stage_ids(path: &ServerSignalPath) -> Vec<&'static str> {
        path.stages.iter().map(|stage| stage.id).collect()
    }

    fn boosted(gain_db: f64, headroom: Option<f64>) -> DspSettings {
        let mut dsp = DspSettings::default();
        dsp.eq.enabled = true;
        dsp.eq.preset = "custom".into();
        dsp.eq.bands[2].gain_db = gain_db;
        if let Some(headroom) = headroom {
            dsp.headroom.enabled = true;
            dsp.headroom.gain_db = headroom;
        }
        dsp
    }

    /// Reads `seconds` of what the opener produces.
    fn run(opener: &mut PcmOpener, frames: usize, frame_bytes: usize) -> Vec<u8> {
        let mut pcm = opener(0).unwrap();
        let mut out = vec![0u8; frames * frame_bytes];
        pcm.read_exact(&mut out).unwrap();
        out
    }

    #[test]
    fn a_track_the_device_takes_plays_bit_perfect_and_a_capped_one_converts() {
        let pipeline = pipeline(None);
        let flat = DspSettings::default();
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &flat,
            Some(&audio(96_000, 24, true)),
            None,
        )
        .unwrap();
        assert!(planned.samples.plan.bit_perfect);
        assert!(planned.samples.processing.is_none());
        assert_eq!(planned.signal_path.quality, SignalQuality::Lossless);
        assert_eq!(planned.input, InputFormat::S32);
        let output = planned.signal_path.stages.last().unwrap();
        assert_eq!(output.id, "native-output");
        assert_eq!(
            planned.signal_path.output_device.as_ref().unwrap().host,
            "studio"
        );
        let capped = OutputSettings {
            max_sample_rate: Some(48_000),
            ..OutputSettings::default()
        };
        let planned = plan(
            &pipeline,
            &capped,
            &flat,
            Some(&audio(96_000, 24, true)),
            None,
        )
        .unwrap();
        assert_eq!(planned.samples.plan.resample, Some((96_000, 48_000)));
        assert_eq!(planned.stream.sample_rate, 48_000);
        assert_eq!(planned.signal_path.quality, SignalQuality::HighQuality);
        let resample = planned
            .signal_path
            .stages
            .iter()
            .find(|stage| stage.id == "native-resample")
            .unwrap();
        assert_eq!(resample.detail, "96kHz to 48kHz (test resampler)");
    }

    #[test]
    fn unknown_formats_and_unprobed_devices_are_refused() {
        let pipeline = pipeline(None);
        let flat = DspSettings::default();
        assert_eq!(
            plan(&pipeline, &OutputSettings::default(), &flat, None, None).err(),
            Some(PlanFailure::UnknownFormat)
        );
        let unprobed = DeviceInfo {
            caps: None,
            ..dac()
        };
        let refused = pipeline.plan(
            &unprobed,
            &OutputSettings::default(),
            &flat,
            TrackSource {
                song_id: "song-1",
                path: "/music/track.flac",
                audio: Some(&audio(44_100, 16, true)),
            },
            None,
            DevicePlace { host: "studio" },
        );
        assert_eq!(refused.err(), Some(PlanFailure::Plan(PlanError::NotProbed)));
    }

    #[test]
    fn leveling_and_the_dsp_chain_run_in_double_precision_and_are_enhanced() {
        let pipeline = pipeline(None);
        let leveling = TrackLeveling {
            gain_db: -6.4,
            source: "album loudness".into(),
        };
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &DspSettings::default(),
            Some(&audio(96_000, 24, true)),
            Some(&leveling),
        )
        .unwrap();
        assert!(!planned.samples.plan.bit_perfect);
        assert_eq!(planned.signal_path.quality, SignalQuality::Enhanced);
        let stage = planned
            .signal_path
            .stages
            .iter()
            .find(|stage| stage.id == "native-leveling")
            .unwrap();
        assert_eq!(stage.detail, "\u{2212}6.4 dB, album loudness");
        // The chain dithers to the device's widest word.
        assert_eq!(planned.samples.plan.bit_depth, 32);
        // A clipping EQ is flagged by the chain's live measurements.
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &boosted(12.0, None),
            Some(&audio(44_100, 16, true)),
            None,
        )
        .unwrap();
        let stats = Arc::new(DspStats::default());
        let mut opener = pipeline.opener(
            PathBuf::from("/music/track.flac"),
            &planned.samples,
            Arc::clone(&stats),
        );
        run(&mut opener, 44_100, 8);
        let status = stats.snapshot();
        assert!(status.clipping && status.clipped_samples > 0, "{status:?}");
        // Headroom keeps it clean.
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &boosted(12.0, Some(-12.0)),
            Some(&audio(44_100, 16, true)),
            None,
        )
        .unwrap();
        assert!(
            planned
                .samples
                .processing
                .as_ref()
                .unwrap()
                .headroom_db
                .is_some()
        );
        let stats = Arc::new(DspStats::default());
        let mut opener = pipeline.opener(
            PathBuf::from("/music/track.flac"),
            &planned.samples,
            Arc::clone(&stats),
        );
        run(&mut opener, 44_100, 8);
        assert!(!stats.snapshot().clipping);
        // Bypassing the chain gives the bit-perfect path back.
        let mut bypassed = boosted(12.0, Some(-12.0));
        bypassed.enabled = false;
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &bypassed,
            Some(&audio(44_100, 16, true)),
            None,
        )
        .unwrap();
        assert!(planned.samples.plan.bit_perfect && planned.samples.processing.is_none());
        assert!(planned.signal_path.processing.is_none());
    }

    fn dsd_audio() -> AudioProperties {
        AudioProperties {
            codec: Some("dsd".into()),
            container: Some("dsf".into()),
            sample_rate: Some(2_822_400),
            bit_depth: Some(1),
            channels: Some(2),
            bitrate_kbps: Some(5_644),
            lossless: Some(true),
        }
    }

    #[test]
    fn dsd_follows_the_device_strategy() {
        let pipeline = pipeline(None);
        let flat = DspSettings::default();
        let dsd = dsd_audio();
        // The simulated DAC reports native DSD_U32_BE up to DSD128.
        let native = plan(
            &pipeline,
            &OutputSettings::default(),
            &flat,
            Some(&dsd),
            None,
        )
        .unwrap();
        let plan_dsd = native.samples.plan.dsd.clone().unwrap();
        assert_eq!(plan_dsd.mode, DsdMode::Native);
        assert_eq!(native.stream.sample_format, SampleFormat::DsdU32Be);
        assert_eq!(native.stream.sample_rate, 88_200);
        assert_eq!(native.input, InputFormat::Raw);
        assert!(native.samples.plan.bit_perfect);
        assert_eq!(native.signal_path.quality, SignalQuality::Lossless);
        assert!(stage_ids(&native.signal_path).contains(&"native-dsd-read"));
        assert!(
            native
                .signal_path
                .stages
                .last()
                .unwrap()
                .detail
                .contains("native DSD64 (DSD_U32_BE")
        );
        assert_eq!(
            native.signal_path.delivered.summary,
            "Native DSD64 2.8MHz 2ch"
        );

        let dop = OutputSettings {
            dsd_strategy: DsdStrategy::Dop,
            ..OutputSettings::default()
        };
        let planned = plan(&pipeline, &dop, &flat, Some(&dsd), None).unwrap();
        assert_eq!(
            planned.samples.plan.dsd.as_ref().unwrap().mode,
            DsdMode::Dop
        );
        assert_eq!(planned.stream.sample_rate, 176_400);
        assert_eq!(planned.stream.sample_format, SampleFormat::S24Packed);
        assert!(stage_ids(&planned.signal_path).contains(&"native-dop"));
        assert_eq!(planned.signal_path.quality, SignalQuality::Lossless);

        let pcm = OutputSettings {
            dsd_strategy: DsdStrategy::Pcm,
            ..OutputSettings::default()
        };
        let planned = plan(&pipeline, &pcm, &flat, Some(&dsd), None).unwrap();
        assert_eq!(
            planned.samples.plan.dsd.as_ref().unwrap().mode,
            DsdMode::Pcm
        );
        // 352.8 kHz from the decoder, converted to the DAC's 176.4 kHz.
        assert_eq!(planned.samples.plan.resample, Some((352_800, 176_400)));
        assert!(stage_ids(&planned.signal_path).contains(&"dsd-to-pcm"));
        assert_eq!(planned.signal_path.quality, SignalQuality::HighQuality);

        // DSP works on PCM, so it converts DSD even under Native.
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &boosted(-3.0, Some(-3.0)),
            Some(&dsd),
            None,
        )
        .unwrap();
        let mode = planned.samples.plan.dsd.as_ref().unwrap();
        assert_eq!(mode.mode, DsdMode::Pcm);
        assert!(mode.fallback_reason.as_deref().unwrap().contains("DSP"));
        assert!(planned.signal_path.processing.is_some());
    }

    #[test]
    fn convolution_filters_are_stored_planned_and_run_by_the_chain() {
        let dir = std::env::temp_dir().join(format!("parson-filters-{}", uuid::Uuid::new_v4()));
        let pipeline = pipeline(Some(dir.join(FILTER_DIR)));
        let store = &pipeline.filters;
        // A 48 kHz filter: 300 samples of delay, then −6 dB.
        let mut taps = vec![0.0; 600];
        taps[300] = 0.5;
        let wav = parson_output::convolution::write_wav_f64(48_000, &[taps]);
        let filter = store.add("", Some("Living room.wav"), &wav).unwrap();
        assert_eq!(filter.record.name, "Living room");
        assert_eq!(filter.summary.taps, 600);
        assert!((filter.summary.peak_gain_db + 6.0).abs() < 0.1);
        assert_eq!(filter.response.len(), 96);
        assert!(store.add("bad", None, b"RIFF....WAVE").is_err());
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.wav(&filter.record.id).unwrap(), wav);
        assert!(store.wav("../index").is_none());
        assert!(store.load("..").is_none());

        let mut dsp = DspSettings::default();
        dsp.convolution.enabled = true;
        dsp.convolution.filter_id = Some(filter.record.id.clone());
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &dsp,
            Some(&audio(44_100, 16, true)),
            None,
        )
        .unwrap();
        let stage = planned
            .signal_path
            .stages
            .iter()
            .find(|stage| stage.id == "native-convolution")
            .expect("a convolution stage");
        assert_eq!(
            stage.detail,
            "Living room: 600 taps at 44.1kHz, converted from 48kHz, one filter for every channel"
        );
        assert_eq!(planned.signal_path.quality, SignalQuality::Enhanced);
        let stats = Arc::new(DspStats::default());
        let mut opener = pipeline.opener(
            PathBuf::from("/music/track.flac"),
            &planned.samples,
            Arc::clone(&stats),
        );
        run(&mut opener, 44_100, 8);
        // The 0.9 sine comes out 6 dB lower.
        let peak = stats.snapshot().peak_dbfs.unwrap();
        assert!((peak - 20.0 * 0.45f64.log10()).abs() < 0.3, "{peak}");

        // Deleting the filter leaves a note instead of a broken chain.
        store.delete(&filter.record.id).unwrap();
        assert!(store.delete(&filter.record.id).is_err());
        let planned = plan(
            &pipeline,
            &OutputSettings::default(),
            &dsp,
            Some(&audio(44_100, 16, true)),
            None,
        )
        .unwrap();
        assert!(!stage_ids(&planned.signal_path).contains(&"native-convolution"));
        assert_eq!(planned.signal_path.quality, SignalQuality::Lossless);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn decode_arguments_only_resample_or_dither_when_the_plan_says_so() {
        let source = SourceFormat {
            sample_rate: 96_000,
            bit_depth: Some(24),
            channels: 2,
            lossless: true,
        };
        let caps = parson_output::simulated::simulated_caps();
        let plan = plan_output(source, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(decode_filter(&plan, None, true), None);
        let joined = decode_args(Path::new("/m/a.flac"), &plan, None, 61_250, true)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-ss 61.250 -i /m/a.flac"));
        assert!(joined.ends_with("-ac 2 -ar 96000 -c:a pcm_s32le -f s32le pipe:1"));
        assert!(!joined.contains("-af"));

        let capped = OutputSettings {
            max_sample_rate: Some(48_000),
            max_bit_depth: Some(16),
            ..OutputSettings::default()
        };
        let plan = plan_output(source, Some(&caps), &capped).unwrap();
        assert_eq!(
            decode_filter(&plan, None, true).as_deref(),
            Some("aresample=osr=48000:resampler=soxr:precision=28:dither_method=triangular")
        );
        assert!(
            decode_filter(&plan, None, false)
                .unwrap()
                .contains("filter_size=64")
        );
        let joined = decode_args(Path::new("/m/a.flac"), &plan, None, 0, true)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!joined.contains("-ss"));
        assert!(joined.ends_with("-c:a pcm_s16le -f s16le pipe:1"));
        let plan = plan_output(source, Some(&caps), &OutputSettings::default()).unwrap();
        assert_eq!(
            decode_filter(&plan, Some(-6.4), true).as_deref(),
            Some("volume=-6.40dB:precision=double,aresample=osr=96000")
        );
        assert_eq!(decode_filter(&plan, Some(0.0), true), None);
    }

    #[test]
    fn source_formats_come_from_indexed_properties() {
        let lossy = source_format(Some(&audio(44_100, 16, false))).unwrap();
        assert_eq!(lossy.bit_depth, None);
        assert!(!lossy.lossless);
        let flac = source_format(Some(&audio(88_200, 24, true))).unwrap();
        assert_eq!(flac.bit_depth, Some(24));
        assert_eq!(flac.sample_rate, 88_200);
        // Indexed before losslessness was recorded: the codec decides.
        let older = AudioProperties {
            lossless: None,
            ..audio(48_000, 24, true)
        };
        assert!(source_format(Some(&older)).unwrap().lossless);
        assert!(source_format(None).is_none());
    }

    #[test]
    fn float_decode_arguments_feed_the_chain() {
        let spec = FloatDecode {
            channels: 2,
            sample_rate: 44_100,
            resample_to: None,
        };
        let joined = float_decode_args(Path::new("/m/a.flac"), &spec, 1_500, true)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-ss 1.500 -i /m/a.flac"));
        assert!(!joined.contains("-af"));
        assert!(joined.ends_with("-ac 2 -ar 44100 -c:a pcm_f64le -f f64le pipe:1"));
        let joined = float_decode_args(
            Path::new("/m/a.flac"),
            &FloatDecode {
                resample_to: Some(768_000),
                ..spec
            },
            0,
            true,
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
        assert!(joined.contains("-af aresample=osr=768000:resampler=soxr"));
        assert!(joined.contains("-ar 768000"));
    }

    #[test]
    fn ffmpeg_decodes_a_real_flac_into_the_chain() {
        // Exercises the production decoder end to end when ffmpeg exists.
        let dir = std::env::temp_dir().join(format!("parson-flac-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let flac = dir.join("tone.flac");
        let made = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("sine=frequency=1000:sample_rate=44100:duration=0.5")
            .args(["-ac", "2", "-sample_fmt", "s16", "-y"])
            .arg(&flac)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        let spec = FloatDecode {
            channels: 2,
            sample_rate: 44_100,
            resample_to: None,
        };
        let processing = ProcessingPlan {
            leveling_db: None,
            headroom_db: Some(-3.0),
            bands: vec![EqBand {
                id: "band-1".into(),
                enabled: true,
                filter_type: EqFilterType::Peaking,
                frequency: 1_000.0,
                gain_db: 3.0,
                q: 1.0,
            }],
            crossfeed: Some(CrossfeedStrength::Medium),
            crossfeed_skipped: false,
            channels: 2,
            input_rate: 44_100,
            output_rate: 48_000,
            rate_requested: true,
            requested_rate_unavailable: None,
            resampler: Some(ResamplerKind::Parson),
            output_bits: 24,
            convolution: None,
            convolution_skipped: None,
        };
        let stats = Arc::new(DspStats::default());
        let decoded = ffmpeg_float_decoder(&flac, &spec, 0).unwrap();
        let mut pcm = DspPcm::new(
            decoded,
            DspChain::new(&processing, 44_100, InputFormat::S32, Arc::clone(&stats)),
        );
        let mut out = Vec::new();
        pcm.read_to_end(&mut out).unwrap();
        // 0.5 s at 48 kHz, stereo, 4-byte samples.
        assert_eq!(out.len(), 24_000 * 2 * 4);
        let status = stats.snapshot();
        assert!(!status.clipping);
        assert!(status.peak_dbfs.unwrap() < 0.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
