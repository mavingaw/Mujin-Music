//! DIDL-Lite track metadata, as Sonos expects it next to each queued URI.

use super::xml::{escape, format_duration};

/// Metadata for one queued track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DidlTrack {
    pub item_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_art_uri: Option<String>,
    pub duration_ms: i64,
    pub uri: String,
    /// MIME type of the stream, e.g. `audio/flac`.
    pub mime_type: String,
}

/// Renders a single-item DIDL-Lite document.
pub fn render(track: &DidlTrack) -> String {
    let mut document = String::with_capacity(768);
    document.push_str(r#"<DIDL-Lite xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns:r="urn:schemas-rinconnetworks-com:metadata-1-0/" xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/">"#);
    document.push_str(&format!(
        r#"<item id="{}" parentID="parson:queue" restricted="true">"#,
        escape(&track.item_id)
    ));
    document.push_str(&format!("<dc:title>{}</dc:title>", escape(&track.title)));
    document.push_str(&format!(
        "<dc:creator>{}</dc:creator>",
        escape(&track.artist)
    ));
    document.push_str(&format!(
        "<upnp:artist>{}</upnp:artist>",
        escape(&track.artist)
    ));
    document.push_str(&format!(
        "<upnp:album>{}</upnp:album>",
        escape(&track.album)
    ));
    if let Some(art) = &track.album_art_uri {
        document.push_str(&format!(
            "<upnp:albumArtURI>{}</upnp:albumArtURI>",
            escape(art)
        ));
    }
    document.push_str("<upnp:class>object.item.audioItem.musicTrack</upnp:class>");
    document.push_str(&format!(
        r#"<res protocolInfo="http-get:*:{}:*" duration="{}">{}</res>"#,
        escape(&track.mime_type),
        format_duration(track.duration_ms),
        escape(&track.uri)
    ));
    document.push_str("</item></DIDL-Lite>");
    document
}

/// Reads `dc:title` back out of DIDL-Lite (Sonos echoes it in `TrackMetaData`).
pub fn title(document: &str) -> Option<String> {
    let document = roxmltree::Document::parse(document).ok()?;
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "title")
        .and_then(|node| node.text())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{DidlTrack, render, title};

    #[test]
    fn renders_escaped_music_track_with_protocol_info() {
        let track = DidlTrack {
            item_id: "parson:song-1".to_string(),
            title: "Rock & <Roll>".to_string(),
            artist: "AC/DC".to_string(),
            album: "\"Live\"".to_string(),
            album_art_uri: Some("http://10.0.0.2:1993/art?a=1&b=2".to_string()),
            duration_ms: 205_000,
            uri: "http://10.0.0.2:1993/s?x=1&y=2".to_string(),
            mime_type: "audio/flac".to_string(),
        };
        let document = render(&track);
        assert!(document.contains("<dc:title>Rock &amp; &lt;Roll&gt;</dc:title>"));
        assert!(document.contains(r#"protocolInfo="http-get:*:audio/flac:*""#));
        assert!(document.contains(r#"duration="0:03:25""#));
        assert!(document.contains(">http://10.0.0.2:1993/s?x=1&amp;y=2</res>"));
        assert!(document.contains("<upnp:class>object.item.audioItem.musicTrack</upnp:class>"));
        roxmltree::Document::parse(&document).expect("well-formed DIDL-Lite");
        assert_eq!(title(&document).as_deref(), Some("Rock & <Roll>"));
    }
}
