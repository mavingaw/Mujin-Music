//! Artwork URLs for provider catalog items.
//!
//! Live mode returns the services' own CDN URLs. Sandbox mode never contacts
//! a CDN: it renders a small deterministic SVG cover from the item's image id
//! so recorded fixtures still look like a catalog.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtworkMode {
    Live,
    Sandbox,
}

impl ArtworkMode {
    /// TIDAL publishes covers by UUID: `ab12-...` becomes
    /// `https://resources.tidal.com/images/ab12/.../640x640.jpg`.
    pub fn tidal(self, uuid: Option<&str>, size: u32) -> Option<String> {
        let uuid = uuid.filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        })?;
        Some(match self {
            Self::Live => format!(
                "https://resources.tidal.com/images/{}/{size}x{size}.jpg",
                uuid.replace('-', "/")
            ),
            Self::Sandbox => sandbox_cover(uuid),
        })
    }

    /// Qobuz returns absolute image URLs.
    pub fn qobuz(self, url: Option<&str>) -> Option<String> {
        let url = url.filter(|value| value.starts_with("https://"))?;
        Some(match self {
            Self::Live => url.to_string(),
            Self::Sandbox => sandbox_cover(url),
        })
    }
}

/// A deterministic two-tone SVG cover, as a `data:` URL.
pub fn sandbox_cover(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let hue = u16::from_be_bytes([digest[0], digest[1]]) % 360;
    let accent = (hue + 40 + u16::from(digest[2]) % 140) % 360;
    let x = 20 + u16::from(digest[3]) % 60;
    let y = 20 + u16::from(digest[4]) % 60;
    let radius = 18 + u16::from(digest[5]) % 30;
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'>\
<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'>\
<stop offset='0' stop-color='hsl({hue},45%,30%)'/>\
<stop offset='1' stop-color='hsl({accent},55%,12%)'/></linearGradient></defs>\
<rect width='100' height='100' fill='url(#g)'/>\
<circle cx='{x}' cy='{y}' r='{radius}' fill='hsl({accent},70%,62%)' opacity='0.55'/>\
<circle cx='{x}' cy='{y}' r='{}' fill='none' stroke='hsl({hue},80%,85%)' stroke-opacity='0.35'/>\
</svg>",
        radius + 9
    );
    format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg))
}

#[cfg(test)]
mod tests {
    use super::ArtworkMode;

    #[test]
    fn tidal_cover_uuids_map_to_cdn_paths() {
        assert_eq!(
            ArtworkMode::Live
                .tidal(Some("4f6c3a2e-1b2c-4d5e-8f90-a1b2c3d4e5f6"), 640)
                .as_deref(),
            Some(
                "https://resources.tidal.com/images/4f6c3a2e/1b2c/4d5e/8f90/a1b2c3d4e5f6/640x640.jpg"
            )
        );
        assert_eq!(ArtworkMode::Live.tidal(Some("../evil"), 640), None);
        assert_eq!(ArtworkMode::Live.qobuz(Some("http://insecure")), None);
    }

    #[test]
    fn sandbox_covers_are_local_and_stable() {
        let first = ArtworkMode::Sandbox
            .tidal(Some("ab-12"), 640)
            .expect("cover");
        assert!(first.starts_with("data:image/svg+xml;base64,"));
        assert_eq!(Some(first), ArtworkMode::Sandbox.tidal(Some("ab-12"), 320));
    }
}
