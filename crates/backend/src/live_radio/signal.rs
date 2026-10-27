//! The server leg of the signal path for a live station.
//!
//! Honest about radio: lossy stations light the path yellow whatever
//! their bitrate, lossless (FLAC) stations purple. When the directory's
//! listing disagrees with what the stream actually carries, the path says
//! so, because Parson reads the format from the audio itself.

use super::relay::{Delivery, Output, SourceFormat, Transport};
use super::station::{Station, codec_is_lossless, codec_label, normalize_codec};
use crate::signal_path::{
    DeliveryMode, ServerSignalPath, SignalFormat, SignalQuality, SignalStage, StageKind,
    StreamTarget, format_rate, overall,
};

fn summary(
    codec: Option<&str>,
    bitrate: Option<u32>,
    rate: Option<u32>,
    depth: Option<u8>,
    channels: Option<u8>,
) -> String {
    let mut parts = Vec::new();
    match codec {
        Some(codec) => parts.push(codec_label(codec)),
        None => parts.push("Unknown format".into()),
    }
    if let Some(bitrate) = bitrate.filter(|_| codec.is_none_or(|codec| !codec_is_lossless(codec))) {
        parts.push(format!("{bitrate} kbps"));
    }
    if let Some(rate) = rate {
        parts.push(format_rate(rate));
    }
    if let Some(depth) = depth {
        parts.push(format!("{depth}bit"));
    }
    match channels {
        Some(1) => parts.push("mono".into()),
        Some(2) => parts.push("stereo".into()),
        Some(count) => parts.push(format!("{count}ch")),
        None => {}
    }
    parts.join(" ")
}

fn signal_format(
    codec: Option<&str>,
    container: Option<&str>,
    bitrate: Option<u32>,
    rate: Option<u32>,
    depth: Option<u8>,
    channels: Option<u8>,
) -> SignalFormat {
    let lossless = codec.map(codec_is_lossless);
    SignalFormat {
        codec: codec.map(str::to_string),
        container: container.map(str::to_string),
        sample_rate: rate,
        bit_depth: depth,
        channels,
        bitrate_kbps: bitrate,
        lossless,
        high_res: lossless == Some(true)
            && (depth.is_some_and(|depth| depth > 16) || rate.is_some_and(|rate| rate > 48_000)),
        summary: summary(codec, bitrate, rate, depth, channels),
    }
}

fn output_label(output: Output) -> &'static str {
    match output {
        Output::Mp3 => "MP3",
        Output::Adts => "AAC (ADTS)",
        Output::Flac => "FLAC",
        Output::Ogg => "Ogg",
        Output::Wav => "WAV",
    }
}

/// Describes the server leg. `format` is what the relay read from the
/// stream (absent before the station has been played); `destination` names
/// the device a conversion was made for.
pub fn describe(
    station: &Station,
    format: Option<&SourceFormat>,
    transport: Option<Transport>,
    delivery: Option<Delivery>,
    target: StreamTarget,
    destination: Option<&str>,
) -> ServerSignalPath {
    let claimed_codec = station.codec.as_deref().and_then(normalize_codec);
    let codec = format
        .and_then(|format| format.codec.clone())
        .or(claimed_codec.clone());
    let bitrate = format
        .and_then(|format| format.bitrate_kbps)
        .or(station.bitrate_kbps);
    let source = signal_format(
        codec.as_deref(),
        format.map(|format| format.container.as_str()),
        bitrate,
        format.and_then(|format| format.sample_rate),
        format.and_then(|format| format.bit_depth),
        format.and_then(|format| format.channels),
    );
    let verified = format.is_some_and(|format| format.verified);
    let mut explanation = match source.lossless {
        Some(true) => "This station broadcasts lossless audio: every bit it sends reaches the next stage unchanged.".to_string(),
        Some(false) => "Internet radio is almost always lossy (MP3, AAC, Opus or Vorbis). Detail was discarded before the stream left the station, and no player can restore it.".to_string(),
        None => "Parson could not identify this stream's codec yet, so it cannot call it lossless.".to_string(),
    };
    if verified {
        explanation.push_str(" Parson read this format from the audio frames themselves, not from the station's listing.");
    } else if format.is_none() {
        explanation.push_str(" This is the directory's listing; Parson confirms it from the audio once the station plays.");
    }
    if let (Some(claimed), Some(actual)) = (claimed_codec.as_deref(), codec.as_deref())
        && verified
        && claimed != actual
    {
        explanation.push_str(&format!(
            " The directory lists {} but the stream carries {}.",
            codec_label(claimed),
            codec_label(actual)
        ));
    }
    if verified
        && let (Some(listed), Some(actual)) = (
            station.bitrate_kbps,
            format.and_then(|format| format.bitrate_kbps),
        )
        && listed.abs_diff(actual) > 16
    {
        explanation.push_str(&format!(
            " The directory lists {listed} kbps; the stream's frames say {actual} kbps."
        ));
    }
    let mut stages = vec![SignalStage {
        id: "source",
        kind: StageKind::Source,
        title: "Live radio".into(),
        detail: format!("{} · {}", station.name, source.summary),
        quality: source.lossless.map(|lossless| {
            if lossless {
                SignalQuality::Lossless
            } else {
                SignalQuality::LowQuality
            }
        }),
        explanation,
    }];
    let (relay_detail, relay_explanation) = match transport {
        Some(Transport::Hls) => (
            "HLS segments, joined live by Parson",
            "Parson fetches the station's HLS playlist on the server, picks its best rendition (lossless first, then the highest bitrate) and joins the segments into one stream. Your device never contacts the station.",
        ),
        _ => (
            "Relayed by Parson",
            "Parson fetches the stream on the server, removes the in-band ICY title data and passes the audio through bit for bit. Your device never contacts the station, and every zone type can play it.",
        ),
    };
    stages.push(SignalStage {
        id: "relay",
        kind: StageKind::Transport,
        title: "Server relay".into(),
        detail: relay_detail.into(),
        quality: Some(SignalQuality::Lossless),
        explanation: relay_explanation.into(),
    });
    let destination = destination.unwrap_or("this player");
    let (mode, delivered, content_type) = match delivery {
        None | Some(Delivery::Original) => (
            DeliveryMode::Original,
            source.clone(),
            format
                .map(|format| match format.container {
                    super::format::Container::Mp3 => "audio/mpeg",
                    super::format::Container::Adts => "audio/aac",
                    super::format::Container::Flac => "audio/flac",
                    super::format::Container::Ogg => "audio/ogg",
                    _ => "application/octet-stream",
                })
                .unwrap_or("application/octet-stream")
                .to_string(),
        ),
        Some(Delivery::Repackage { output }) => {
            stages.push(SignalStage {
                id: "repackage",
                kind: StageKind::Conversion,
                title: "Repackaged".into(),
                detail: format!("Same audio in a {} stream, no re-encode", output_label(output)),
                quality: Some(SignalQuality::Lossless),
                explanation: "The segments' audio is copied into a continuous stream your player understands. The encoded audio is not touched.".into(),
            });
            let mut delivered = source.clone();
            delivered.container = Some(output.codec().into());
            (
                DeliveryMode::Original,
                delivered,
                output.content_type().to_string(),
            )
        }
        Some(Delivery::Convert {
            output,
            sample_rate,
            bit_depth,
            bitrate_kbps,
        }) => {
            let lossless_output = matches!(output, Output::Flac | Output::Wav);
            let rate = sample_rate.or(source.sample_rate);
            let delivered = signal_format(
                Some(output.codec()),
                Some(output.codec()),
                bitrate_kbps,
                rate,
                bit_depth,
                source.channels,
            );
            if lossless_output {
                stages.push(SignalStage {
                    id: "decode",
                    kind: StageKind::Conversion,
                    title: format!("Decoded to {}", output_label(output)),
                    detail: format!(
                        "For {destination}, which cannot play {}{}",
                        codec.as_deref().map(codec_label).unwrap_or_else(|| "this format".into()),
                        sample_rate
                            .map(|rate| format!(" at this rate; resampled to {}", format_rate(rate)))
                            .unwrap_or_default()
                    ),
                    quality: Some(SignalQuality::HighQuality),
                    explanation: "Parson decodes the stream once and sends the samples losslessly, so no second lossy generation is added.".into(),
                });
            } else {
                stages.push(SignalStage {
                    id: "encode",
                    kind: StageKind::Encode,
                    title: format!("Re-encoded to {}", output_label(output)),
                    detail: format!(
                        "{} kbps for {destination}, which only plays {}",
                        bitrate_kbps.unwrap_or(320),
                        output_label(output)
                    ),
                    quality: Some(SignalQuality::LowQuality),
                    explanation: "This output cannot take the station's format or a lossless one, so Parson encodes it again. A second lossy generation loses a little more detail.".into(),
                });
            }
            (
                DeliveryMode::Transcode,
                delivered,
                output.content_type().to_string(),
            )
        }
    };
    let quality = overall(&stages);
    ServerSignalPath {
        song_id: station.song_id(),
        target,
        mode,
        quality,
        source,
        delivered,
        content_type,
        requested_bitrate_kbps: 0,
        effect: None,
        stages,
        processing: None,
        output_device: None,
        hardware_confirmed: None,
    }
}

#[cfg(test)]
mod tests {
    use super::describe;
    use crate::live_radio::format::Container;
    use crate::live_radio::relay::{Delivery, Output, SourceFormat, Transport};
    use crate::live_radio::store::tests::station;
    use crate::signal_path::{SignalQuality, StreamTarget};

    fn heard(codec: &str, bitrate: Option<u32>, container: Container) -> SourceFormat {
        SourceFormat {
            container,
            codec: Some(codec.into()),
            sample_rate: Some(44_100),
            bit_depth: if codec == "flac" { Some(16) } else { None },
            channels: Some(2),
            bitrate_kbps: bitrate,
            verified: true,
        }
    }

    #[test]
    fn lossy_stations_are_yellow_whatever_the_bitrate() {
        let station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "Jazz");
        let path = describe(
            &station,
            Some(&heard("mp3", Some(320), Container::Mp3)),
            Some(Transport::Http),
            Some(Delivery::Original),
            StreamTarget::Direct,
            None,
        );
        assert_eq!(path.quality, SignalQuality::LowQuality);
        assert_eq!(path.stages[0].detail, "Jazz · MP3 320 kbps 44.1kHz stereo");
        assert!(path.stages[0].explanation.contains("lists 128 kbps"));
        assert_eq!(path.song_id, "radio:d1a54d2e-623e-4970-ab11-35f7b56c5ec3");
    }

    #[test]
    fn lossless_stations_stay_purple_until_something_converts_them() {
        let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "Paradise");
        station.codec = Some("flac".into());
        let flac = heard("flac", None, Container::Flac);
        let direct = describe(
            &station,
            Some(&flac),
            Some(Transport::Http),
            Some(Delivery::Original),
            StreamTarget::Direct,
            None,
        );
        assert_eq!(direct.quality, SignalQuality::Lossless);
        assert_eq!(direct.source.summary, "FLAC 44.1kHz 16bit stereo");

        let hls = describe(
            &station,
            Some(&heard("flac", None, Container::Fmp4)),
            Some(Transport::Hls),
            Some(Delivery::Repackage {
                output: Output::Flac,
            }),
            StreamTarget::Direct,
            None,
        );
        assert_eq!(hls.quality, SignalQuality::Lossless);
        assert!(hls.stages.iter().any(|stage| stage.id == "repackage"));

        let mp3_only = describe(
            &station,
            Some(&flac),
            Some(Transport::Http),
            Some(Delivery::Convert {
                output: Output::Mp3,
                sample_rate: None,
                bit_depth: None,
                bitrate_kbps: Some(320),
            }),
            StreamTarget::Cast,
            Some("Kitchen speaker"),
        );
        assert_eq!(mp3_only.quality, SignalQuality::LowQuality);
        assert!(
            mp3_only
                .stages
                .last()
                .unwrap()
                .detail
                .contains("Kitchen speaker")
        );
    }

    #[test]
    fn unplayed_stations_use_the_listing_and_say_so() {
        let mut station = station("d1a54d2e-623e-4970-ab11-35f7b56c5ec3", "Talk");
        station.codec = Some("AAC+".into());
        let path = describe(&station, None, None, None, StreamTarget::Direct, None);
        assert_eq!(path.source.codec.as_deref(), Some("aac"));
        assert!(path.stages[0].explanation.contains("directory's listing"));
        station.codec = None;
        let unknown = describe(&station, None, None, None, StreamTarget::Direct, None);
        assert_eq!(
            unknown.quality,
            SignalQuality::HighQuality.min(unknown.quality)
        );
        assert!(unknown.stages[0].quality.is_none());
    }
}
