//! UPnP device description (`/xml/device_description.xml`) parsing.

use super::xml::child_text;

/// The parts of a Sonos device description Parson uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceDescription {
    /// Player id without the `uuid:` prefix, e.g. `RINCON_000E58A1B2C301400`.
    pub uuid: String,
    /// Room name set in the Sonos app.
    pub room_name: String,
    /// Marketing model name, e.g. `Sonos Play:5`.
    pub model_name: String,
    /// Short model name, e.g. `Play:5`.
    pub display_name: String,
    pub model_number: Option<String>,
    pub software_version: Option<String>,
}

/// Parses a ZonePlayer device description.
pub fn parse_device_description(document: &str) -> Result<DeviceDescription, String> {
    let document = roxmltree::Document::parse(document).map_err(|error| error.to_string())?;
    let device = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "device"
                && child_text(*node, "deviceType").is_some_and(|kind| kind.contains("ZonePlayer"))
        })
        .ok_or_else(|| "not a Sonos ZonePlayer description".to_string())?;
    let direct = |name: &str| {
        device
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == name)
            .and_then(|child| child.text())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    let uuid = direct("UDN")
        .map(|udn| udn.trim_start_matches("uuid:").to_string())
        .filter(|uuid| valid_player_uuid(uuid))
        .ok_or_else(|| "device description has no valid UDN".to_string())?;
    let model_name = direct("modelName").unwrap_or_else(|| "Sonos".to_string());
    Ok(DeviceDescription {
        room_name: direct("roomName").unwrap_or_else(|| model_name.clone()),
        display_name: direct("displayName").unwrap_or_else(|| model_name.clone()),
        model_name,
        model_number: direct("modelNumber"),
        software_version: direct("softwareVersion"),
        uuid,
    })
}

/// Sonos player ids are `RINCON_` plus hex/alphanumerics.
pub fn valid_player_uuid(value: &str) -> bool {
    value.len() <= 64
        && value.starts_with("RINCON_")
        && value.len() > "RINCON_".len()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::{parse_device_description, valid_player_uuid};

    #[test]
    fn reads_room_model_and_player_id() {
        let xml = r#"<?xml version="1.0" encoding="utf-8" ?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:ZonePlayer:1</deviceType>
    <friendlyName>192.168.1.20 - Sonos Play:5 - RINCON_000E58A1B2C301400</friendlyName>
    <manufacturer>Sonos, Inc.</manufacturer>
    <modelNumber>S6</modelNumber>
    <modelName>Sonos Play:5</modelName>
    <displayName>Play:5</displayName>
    <softwareVersion>79.1-56030</softwareVersion>
    <roomName>Living Room &amp; Kitchen</roomName>
    <UDN>uuid:RINCON_000E58A1B2C301400</UDN>
    <deviceList>
      <device>
        <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
        <UDN>uuid:RINCON_000E58A1B2C301400_MR</UDN>
      </device>
    </deviceList>
  </device>
</root>"#;
        let description = parse_device_description(xml).expect("description");
        assert_eq!(description.uuid, "RINCON_000E58A1B2C301400");
        assert_eq!(description.room_name, "Living Room & Kitchen");
        assert_eq!(description.model_name, "Sonos Play:5");
        assert_eq!(description.display_name, "Play:5");
        assert_eq!(description.model_number.as_deref(), Some("S6"));
    }

    #[test]
    fn rejects_non_sonos_devices_and_bad_ids() {
        assert!(
            parse_device_description(
                "<root><device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType><UDN>uuid:x</UDN></device></root>"
            )
            .is_err()
        );
        assert!(valid_player_uuid("RINCON_ABC123"));
        assert!(!valid_player_uuid("RINCON_"));
        assert!(!valid_player_uuid("RINCON_../../etc"));
        assert!(!valid_player_uuid("uuid:RINCON_A"));
    }
}
