//! The device engine on simulated DACs, through the server's pipeline:
//! native DSD and DoP read from the file and written untouched, the
//! driver's own report confirming (or breaking) the path, and the DSP
//! chain's live measurements in the zone's signal path.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parson_output::dsd::{DsdPacking, DsdPcm, DsdReader};
use parson_output::dsp::DspSettings;
use parson_output::procfs::{HardwareReporter, HardwareStatus};
use parson_output::simulated::{SIMULATED_DEVICE_ID, SimulatedBackend};
use parson_output::{Backend, DeviceInfo, Driver, OutputSettings, SampleFormat, StreamFormat};

use super::{DeviceManager, Placement};
use crate::domain::AudioProperties;
use crate::outputs::renderer::{
    PlayState, Renderer, RendererSettings, RendererState, SourceFormat, TrackRequest,
};
use crate::signal_path::SignalQuality;
use crate::zones::VolumeMode;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("parson-engine-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn track(path: &Path, audio: AudioProperties, dsp: DspSettings) -> TrackRequest {
    TrackRequest {
        serial: 1,
        song_id: "song-1".into(),
        path: path.to_string_lossy().into_owned(),
        duration_ms: 500,
        source: SourceFormat {
            codec: audio.codec.clone(),
            sample_rate: audio.sample_rate,
            bit_depth: audio.bit_depth,
            channels: audio.channels,
        },
        audio: Some(audio),
        uri: String::new(),
        metadata: String::new(),
        mime_type: String::new(),
        title: "Track".into(),
        artist: "Artist".into(),
        album: String::new(),
        art_uri: None,
        leveling: None,
        dsp,
    }
}

fn manager(backend: Arc<dyn Backend>) -> DeviceManager {
    DeviceManager::placed(
        backend,
        None,
        Placement::local(),
        tokio::sync::broadcast::channel(64).0,
    )
}

fn wait(renderer: &dyn Renderer, done: impl Fn(&RendererState) -> bool) -> RendererState {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = renderer.state();
        if done(&state) || Instant::now() > deadline {
            return state;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn configure(renderer: &dyn Renderer, output: OutputSettings) {
    let info = renderer.info();
    renderer.configure(RendererSettings {
        output,
        volume_mode: VolumeMode::Fixed,
        limits: info.limits,
    });
}

/// 0.125 s of DSD64 in a DSF file, and the bytes `DsdReader` returns.
fn dsd_file(dir: &Path) -> (PathBuf, AudioProperties) {
    let bytes: Vec<u8> = (0..88_200_u32).map(|n| (n * 37 % 251) as u8).collect();
    let path = dir.join("track.dsf");
    std::fs::write(&path, parson_output::dsd::write_dsf(2_822_400, 2, &bytes)).unwrap();
    let audio = AudioProperties {
        codec: Some("dsd".into()),
        container: Some("dsf".into()),
        sample_rate: Some(2_822_400),
        bit_depth: Some(1),
        channels: Some(2),
        bitrate_kbps: Some(5_644),
        lossless: Some(true),
    };
    (path, audio)
}

/// What the device should receive: the file's DSD, packed for it.
fn packed(path: &Path, packing: DsdPacking) -> Vec<u8> {
    let file = std::fs::read(path).unwrap();
    let mut cursor = Cursor::new(file);
    let info = parson_output::dsd::read_info(&mut cursor).unwrap();
    let mut pcm = DsdPcm::new(DsdReader::new(cursor, info, 0), info.channels, packing);
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut pcm, &mut out).unwrap();
    out
}

#[tokio::test]
async fn dsd_reaches_the_dac_natively_or_as_dop_bit_for_bit() {
    let dir = scratch("dsd");
    let (path, audio) = dsd_file(&dir);
    let backend = Arc::new(SimulatedBackend::new(
        SimulatedBackend::dac_up_to_192k().devices(),
        8.0,
    ));
    let devices = manager(backend.clone());
    let renderer = devices
        .renderer(&format!("device:{SIMULATED_DEVICE_ID}"))
        .expect("zone");
    configure(renderer.as_ref(), OutputSettings::default());
    renderer
        .load(
            track(&path, audio.clone(), DspSettings::default()),
            None,
            0,
            true,
        )
        .await
        .unwrap();
    let playing = wait(renderer.as_ref(), |state| state.stream.is_some());
    let stream = playing.stream.expect("signal path");
    let signal = stream.signal_path.as_ref().expect("pipeline path");
    assert!(stream.bit_perfect, "{stream:?}");
    assert_eq!(signal.quality, SignalQuality::Lossless);
    assert!(
        signal
            .stages
            .iter()
            .any(|stage| stage.id == "native-dsd-read")
    );
    assert_eq!(
        stream.output,
        "DSD64 (DSD_U32_BE, 88.2 kHz frames) to Simulated USB DAC, exclusive"
    );
    let ended = wait(renderer.as_ref(), |state| state.finished_serial.is_some());
    assert_eq!(ended.state, PlayState::Stopped);
    let capture = backend.capture(SIMULATED_DEVICE_ID);
    assert_eq!(capture.opened[0].0.sample_format, SampleFormat::DsdU32Be);
    assert_eq!(
        capture.bytes,
        packed(&path, DsdPacking::Native(SampleFormat::DsdU32Be)),
        "the DSD stream arrives untouched"
    );

    // DoP: the same bits under PCM markers.
    let backend = Arc::new(SimulatedBackend::new(
        SimulatedBackend::dac_up_to_192k().devices(),
        8.0,
    ));
    let devices = manager(backend.clone());
    let renderer = devices
        .renderer(&format!("device:{SIMULATED_DEVICE_ID}"))
        .expect("zone");
    configure(
        renderer.as_ref(),
        OutputSettings {
            dsd_strategy: parson_output::DsdStrategy::Dop,
            ..OutputSettings::default()
        },
    );
    renderer
        .load(track(&path, audio, DspSettings::default()), None, 0, true)
        .await
        .unwrap();
    wait(renderer.as_ref(), |state| state.finished_serial.is_some());
    let capture = backend.capture(SIMULATED_DEVICE_ID);
    assert_eq!(capture.opened[0].0.sample_rate, 176_400);
    assert_eq!(capture.opened[0].0.sample_format, SampleFormat::S24Packed);
    assert_eq!(
        capture.bytes,
        packed(&path, DsdPacking::Dop(SampleFormat::S24Packed))
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The simulated DAC with a driver that reports what the hardware runs.
struct ReportingBackend {
    report: Option<&'static str>,
    inner: SimulatedBackend,
}

impl Backend for ReportingBackend {
    fn driver(&self) -> Driver {
        Driver::Simulated
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        self.inner.devices()
    }

    fn open(
        &self,
        device_id: &str,
        format: StreamFormat,
        exclusive: bool,
    ) -> std::io::Result<Box<dyn parson_output::Sink>> {
        self.inner.open(device_id, format, exclusive)
    }

    fn hardware_status(&self, _device_id: &str, stream: &StreamFormat) -> Option<HardwareStatus> {
        let params = parson_output::procfs::parse_hw_params(self.report?)?;
        Some(HardwareStatus {
            source: "/proc/asound/card1/pcm0p/sub0/hw_params".into(),
            reporter: HardwareReporter::Kernel,
            chain: None,
            matches_stream: parson_output::procfs::params_match(&params, stream),
            params,
            momentary_hz: Some(96_000),
            kernel_emulator: false,
        })
    }
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A 1 kHz tone at 0.9 of full scale, as 24-bit FLAC.
fn tone(dir: &Path, rate: u32, seconds: f32) -> (PathBuf, AudioProperties) {
    let path = dir.join(format!("tone-{rate}.flac"));
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!(
                "aevalsrc=0.9*sin(2*PI*1000*t):s={rate}:d={seconds}"
            ))
            .args([
                "-ac",
                "2",
                "-c:a",
                "flac",
                "-sample_fmt",
                "s32",
                "-bits_per_raw_sample",
                "24",
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let audio = AudioProperties {
        codec: Some("flac".into()),
        container: Some("flac".into()),
        sample_rate: Some(rate),
        bit_depth: Some(24),
        channels: Some(2),
        bitrate_kbps: None,
        lossless: Some(true),
    };
    (path, audio)
}

#[tokio::test]
async fn the_driver_report_confirms_or_breaks_the_path() {
    if !ffmpeg_available() {
        return;
    }
    let dir = scratch("report");
    let (path, audio) = tone(&dir, 96_000, 2.0);
    for (report, quality, confirmed) in [
        (
            Some(
                "access: RW_INTERLEAVED\nformat: S24_3LE\nsubformat: STD\nchannels: 2\nrate: 96000 (96000/1)\n",
            ),
            SignalQuality::Lossless,
            Some(true),
        ),
        // Something below Parson (a misconfigured plug, a driver quirk)
        // runs the device at another rate: the path says so.
        (
            Some(
                "access: RW_INTERLEAVED\nformat: S16_LE\nsubformat: STD\nchannels: 2\nrate: 48000 (48000/1)\n",
            ),
            SignalQuality::HighQuality,
            Some(false),
        ),
        (None, SignalQuality::Lossless, None),
    ] {
        let devices = manager(Arc::new(ReportingBackend {
            report,
            inner: SimulatedBackend::dac_up_to_192k(),
        }));
        let renderer = devices
            .renderer(&format!("device:{SIMULATED_DEVICE_ID}"))
            .expect("zone");
        configure(renderer.as_ref(), OutputSettings::default());
        renderer
            .load(
                track(&path, audio.clone(), DspSettings::default()),
                None,
                0,
                true,
            )
            .await
            .unwrap();
        let state = wait(renderer.as_ref(), |state| {
            state.position_ms > 100
                && state
                    .stream
                    .as_ref()
                    .and_then(|stream| stream.signal_path.as_ref())
                    .is_some_and(|path| report.is_none() || path.hardware_confirmed.is_some())
        });
        let stream = state.stream.expect("signal path");
        let path = stream.signal_path.as_ref().expect("pipeline path");
        assert_eq!(path.quality, quality, "{report:?}");
        assert_eq!(path.hardware_confirmed, confirmed);
        let hardware = path
            .stages
            .iter()
            .find(|stage| stage.id == "native-hardware");
        match confirmed {
            Some(true) => {
                let stage = hardware.expect("a driver stage");
                assert_eq!(stage.title, "Confirmed by the driver");
                assert_eq!(
                    stage.detail,
                    "hardware running at S24_3LE 96kHz 2ch, USB clock 96kHz"
                );
                assert!(stream.bit_perfect);
            }
            Some(false) => assert!(
                !stream.bit_perfect,
                "a converting driver is not bit-perfect"
            ),
            None => assert!(hardware.is_none()),
        }
        renderer.stop().await.unwrap();
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn the_dsp_chain_reports_its_speed_and_clipping_while_it_plays() {
    if !ffmpeg_available() {
        return;
    }
    let dir = scratch("dsp");
    let (path, audio) = tone(&dir, 44_100, 4.0);
    let devices = manager(Arc::new(SimulatedBackend::dac_up_to_192k()));
    let renderer = devices
        .renderer(&format!("device:{SIMULATED_DEVICE_ID}"))
        .expect("zone");
    configure(renderer.as_ref(), OutputSettings::default());
    // +12 dB at 1 kHz on a 0.9 full-scale 1 kHz tone clips.
    let mut boosted = DspSettings::default();
    boosted.eq.enabled = true;
    boosted.eq.preset = "custom".into();
    boosted.eq.bands[2].gain_db = 12.0;
    renderer
        .load(track(&path, audio, boosted), None, 0, true)
        .await
        .unwrap();
    let state = wait(renderer.as_ref(), |state| {
        state
            .stream
            .as_ref()
            .and_then(|stream| stream.signal_path.as_ref())
            .and_then(|path| path.processing.as_ref())
            .and_then(|processing| processing.status.as_ref())
            .is_some_and(|status| status.clipping && status.realtime_factor.is_some())
    });
    let stream = state.stream.expect("signal path");
    assert!(!stream.bit_perfect);
    assert_eq!(stream.quality, crate::zones::StreamQuality::Enhanced);
    let processing = stream
        .signal_path
        .as_ref()
        .and_then(|path| path.processing.as_ref())
        .expect("the server's chain");
    let status = processing.status.as_ref().expect("live measurements");
    assert!(status.clipping && status.clipped_samples > 0);
    assert!(status.realtime_factor.unwrap() > 1.0);
    assert!(
        stream
            .steps
            .iter()
            .any(|step| step.starts_with("Parametric EQ: 1 band: 1 kHz peak +12.0 dB")),
        "{:?}",
        stream.steps
    );
    renderer.stop().await.unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
