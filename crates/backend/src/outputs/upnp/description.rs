//! UPnP device and service descriptions of DLNA media renderers.

use crate::sonos::xml::child_text;

pub const AV_TRANSPORT: &str = "urn:schemas-upnp-org:service:AVTransport:1";
pub const RENDERING_CONTROL: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
pub const CONNECTION_MANAGER: &str = "urn:schemas-upnp-org:service:ConnectionManager:1";

/// One service of a renderer, with absolute URLs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceUrls {
    pub service_type: String,
    pub control_url: String,
    pub scpd_url: String,
    pub event_url: Option<String>,
}

/// The parts of a MediaRenderer description Parson uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererDescription {
    /// UDN without the `uuid:` prefix.
    pub udn: String,
    pub friendly_name: String,
    pub manufacturer: String,
    pub model_name: String,
    pub model_number: Option<String>,
    pub av_transport: ServiceUrls,
    pub rendering_control: Option<ServiceUrls>,
    pub connection_manager: Option<ServiceUrls>,
}

impl RendererDescription {
    /// Sonos players describe a MediaRenderer too; they get their own
    /// (richer) zones, so the generic renderer skips them.
    pub fn is_sonos(&self) -> bool {
        self.manufacturer.to_ascii_lowercase().contains("sonos") || self.udn.starts_with("RINCON_")
    }

    /// e.g. `Denon AVR-X2800H`.
    pub fn model(&self) -> String {
        let manufacturer = self.manufacturer.trim();
        let model = self.model_name.trim();
        if manufacturer.is_empty() || model.starts_with(manufacturer) {
            model.to_string()
        } else {
            format!("{manufacturer} {model}")
        }
    }
}

/// Resolves `value` against the description's location (or `URLBase`).
pub fn absolute_url(base: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        return Some(value.to_string());
    }
    let rest = base.strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    if value.starts_with('/') {
        Some(format!("http://{authority}{value}"))
    } else {
        // Relative to the description's directory.
        let path = &rest[authority.len()..];
        let directory = match path.rfind('/') {
            Some(index) => &path[..=index],
            None => "/",
        };
        Some(format!("http://{authority}{directory}{value}"))
    }
}

/// Parses a device description fetched from `location`.
pub fn parse(document: &str, location: &str) -> Result<RendererDescription, String> {
    let document = roxmltree::Document::parse(document).map_err(|error| error.to_string())?;
    let base = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "URLBase")
        .and_then(|node| node.text())
        .map(|text| text.trim().to_string())
        .filter(|text| text.starts_with("http://"))
        .unwrap_or_else(|| location.to_string());
    let device = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "device"
                && child_text(*node, "deviceType")
                    .is_some_and(|kind| kind.contains("device:MediaRenderer"))
        })
        .ok_or_else(|| "not a UPnP MediaRenderer description".to_string())?;
    let direct = |name: &str| {
        device
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == name)
            .and_then(|child| child.text())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    let udn = direct("UDN")
        .map(|udn| udn.trim_start_matches("uuid:").to_string())
        .filter(|udn| valid_udn(udn))
        .ok_or_else(|| "renderer description has no valid UDN".to_string())?;
    let services: Vec<ServiceUrls> = device
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "serviceList")
        .flat_map(|list| list.children().filter(|node| node.is_element()))
        .filter_map(|service| {
            let text = |name: &str| child_text(service, name).filter(|value| !value.is_empty());
            Some(ServiceUrls {
                service_type: text("serviceType")?,
                control_url: absolute_url(&base, &text("controlURL")?)?,
                scpd_url: absolute_url(&base, &text("SCPDURL")?)?,
                event_url: text("eventSubURL").and_then(|url| absolute_url(&base, &url)),
            })
        })
        .collect();
    let find = |prefix: &str| {
        services
            .iter()
            .find(|service| service.service_type.starts_with(prefix))
            .cloned()
    };
    let av_transport = find("urn:schemas-upnp-org:service:AVTransport:")
        .ok_or_else(|| "renderer has no AVTransport service".to_string())?;
    let model_name = direct("modelName").unwrap_or_else(|| "Media renderer".to_string());
    Ok(RendererDescription {
        friendly_name: direct("friendlyName").unwrap_or_else(|| model_name.clone()),
        manufacturer: direct("manufacturer").unwrap_or_default(),
        model_number: direct("modelNumber"),
        model_name,
        av_transport,
        rendering_control: find("urn:schemas-upnp-org:service:RenderingControl:"),
        connection_manager: find("urn:schemas-upnp-org:service:ConnectionManager:"),
        udn,
    })
}

/// UDNs become zone ids and URL path segments: keep them tame.
pub fn valid_udn(value: &str) -> bool {
    (1..=96).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Volume range and optional actions from a service description (SCPD).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceInfo {
    pub actions: Vec<String>,
    /// `Volume` state variable range: minimum, maximum, step.
    pub volume_range: Option<(f32, f32, f32)>,
}

impl ServiceInfo {
    pub fn has_action(&self, name: &str) -> bool {
        self.actions.iter().any(|action| action == name)
    }
}

pub fn parse_scpd(document: &str) -> Result<ServiceInfo, String> {
    let document = roxmltree::Document::parse(document).map_err(|error| error.to_string())?;
    let actions = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "action")
        .filter_map(|action| {
            action
                .children()
                .find(|child| child.is_element() && child.tag_name().name() == "name")
                .and_then(|name| name.text())
                .map(|name| name.trim().to_string())
        })
        .collect();
    let volume_range = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "stateVariable")
        .find(|variable| {
            variable
                .children()
                .find(|child| child.is_element() && child.tag_name().name() == "name")
                .and_then(|name| name.text())
                .is_some_and(|name| name.trim() == "Volume")
        })
        .and_then(|variable| {
            let range = variable
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "allowedValueRange")?;
            let value = |name: &str| -> Option<f32> { child_text(range, name)?.parse().ok() };
            let minimum = value("minimum")?;
            let maximum = value("maximum")?;
            let step = value("step").filter(|step| *step > 0.0).unwrap_or(1.0);
            (maximum > minimum).then_some((minimum, maximum, step))
        });
    Ok(ServiceInfo {
        actions,
        volume_range,
    })
}

#[cfg(test)]
mod tests {
    use super::{absolute_url, parse, parse_scpd, valid_udn};

    #[test]
    fn reads_a_renderer_description_with_relative_urls() {
        let xml = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device>
    <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
    <friendlyName>Living Room AVR</friendlyName>
    <manufacturer>Denon</manufacturer>
    <modelName>AVR-X2800H</modelName>
    <UDN>uuid:5f9ec1b3-ed59-1900-4530-0005cd123456</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <controlURL>/upnp/control/AVTransport1</controlURL>
        <eventSubURL>/upnp/event/AVTransport1</eventSubURL>
        <SCPDURL>AVTransport.xml</SCPDURL>
      </service>
      <service>
        <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
        <controlURL>http://10.0.0.9:60006/upnp/control/RenderingControl1</controlURL>
        <SCPDURL>/RenderingControl.xml</SCPDURL>
      </service>
    </serviceList>
  </device>
</root>"#;
        let description = parse(xml, "http://10.0.0.9:60006/upnp/desc/description.xml")
            .expect("renderer description");
        assert_eq!(description.udn, "5f9ec1b3-ed59-1900-4530-0005cd123456");
        assert_eq!(description.friendly_name, "Living Room AVR");
        assert_eq!(description.model(), "Denon AVR-X2800H");
        assert_eq!(
            description.av_transport.control_url,
            "http://10.0.0.9:60006/upnp/control/AVTransport1"
        );
        assert_eq!(
            description.av_transport.scpd_url,
            "http://10.0.0.9:60006/upnp/desc/AVTransport.xml"
        );
        assert_eq!(
            description
                .rendering_control
                .as_ref()
                .expect("rendering")
                .scpd_url,
            "http://10.0.0.9:60006/RenderingControl.xml"
        );
        assert!(description.connection_manager.is_none());
        assert!(!description.is_sonos());
        assert!(parse("<root><device><deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType></device></root>", "http://h/").is_err());
        assert!(!valid_udn("../etc"));
        assert_eq!(absolute_url("file:///x", "/a"), None);
    }

    #[test]
    fn reads_actions_and_the_volume_range() {
        let scpd = r#"<scpd xmlns="urn:schemas-upnp-org:service-1-0">
  <actionList>
    <action><name>SetVolume</name></action>
    <action><name>GetVolumeDBRange</name></action>
  </actionList>
  <serviceStateTable>
    <stateVariable sendEvents="no"><name>Mute</name><dataType>boolean</dataType></stateVariable>
    <stateVariable sendEvents="no"><name>Volume</name><dataType>ui2</dataType>
      <allowedValueRange><minimum>0</minimum><maximum>30</maximum><step>1</step></allowedValueRange>
    </stateVariable>
  </serviceStateTable>
</scpd>"#;
        let info = parse_scpd(scpd).expect("scpd");
        assert!(info.has_action("GetVolumeDBRange"));
        assert!(!info.has_action("SetNextAVTransportURI"));
        assert_eq!(info.volume_range, Some((0.0, 30.0, 1.0)));
    }
}
