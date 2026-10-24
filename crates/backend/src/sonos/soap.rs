//! UPnP SOAP control for Sonos services.

use std::collections::HashMap;
use std::time::Duration;

use super::xml::escape;

const MAX_SOAP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const SOAP_TIMEOUT: Duration = Duration::from_secs(4);

/// UPnP services Parson drives on a Sonos player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    AVTransport,
    RenderingControl,
    GroupRenderingControl,
    ZoneGroupTopology,
}

impl Service {
    pub fn urn(self) -> &'static str {
        match self {
            Self::AVTransport => "urn:schemas-upnp-org:service:AVTransport:1",
            Self::RenderingControl => "urn:schemas-upnp-org:service:RenderingControl:1",
            Self::GroupRenderingControl => "urn:schemas-upnp-org:service:GroupRenderingControl:1",
            Self::ZoneGroupTopology => "urn:schemas-upnp-org:service:ZoneGroupTopology:1",
        }
    }

    pub fn control_path(self) -> &'static str {
        match self {
            Self::AVTransport => "/MediaRenderer/AVTransport/Control",
            Self::RenderingControl => "/MediaRenderer/RenderingControl/Control",
            Self::GroupRenderingControl => "/MediaRenderer/GroupRenderingControl/Control",
            Self::ZoneGroupTopology => "/ZoneGroupTopology/Control",
        }
    }

    pub fn event_path(self) -> &'static str {
        match self {
            Self::AVTransport => "/MediaRenderer/AVTransport/Event",
            Self::RenderingControl => "/MediaRenderer/RenderingControl/Event",
            Self::GroupRenderingControl => "/MediaRenderer/GroupRenderingControl/Event",
            Self::ZoneGroupTopology => "/ZoneGroupTopology/Event",
        }
    }

    /// Resolves a control path back to its service.
    pub fn from_control_path(path: &str) -> Option<Self> {
        [
            Self::AVTransport,
            Self::RenderingControl,
            Self::GroupRenderingControl,
            Self::ZoneGroupTopology,
        ]
        .into_iter()
        .find(|service| service.control_path() == path)
    }

    pub fn from_event_path(path: &str) -> Option<Self> {
        [
            Self::AVTransport,
            Self::RenderingControl,
            Self::GroupRenderingControl,
            Self::ZoneGroupTopology,
        ]
        .into_iter()
        .find(|service| service.event_path() == path)
    }
}

/// A failed SOAP call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoapError {
    /// The device answered with a UPnP fault (`errorCode`).
    Fault { code: u32, description: String },
    /// Transport, HTTP or parse failure.
    Transport(String),
}

impl std::fmt::Display for SoapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fault { code, description } if description.is_empty() => {
                write!(formatter, "UPnP error {code}")
            }
            Self::Fault { code, description } => {
                write!(formatter, "UPnP error {code}: {description}")
            }
            Self::Transport(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SoapError {}

/// Builds the SOAP envelope for `action` with ordered `arguments`.
pub fn envelope(service: Service, action: &str, arguments: &[(&str, &str)]) -> String {
    let mut body = String::with_capacity(256);
    body.push_str(r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body>"#);
    body.push_str(&format!(r#"<u:{action} xmlns:u="{}">"#, service.urn()));
    for (name, value) in arguments {
        body.push_str(&format!("<{name}>{}</{name}>", escape(value)));
    }
    body.push_str(&format!("</u:{action}></s:Body></s:Envelope>"));
    body
}

/// The `SOAPACTION` header value.
pub fn soap_action_header(service: Service, action: &str) -> String {
    format!("\"{}#{action}\"", service.urn())
}

/// Parses `<u:{action}Response>` output arguments, or the UPnP fault.
pub fn parse_response(document: &str, action: &str) -> Result<HashMap<String, String>, SoapError> {
    let document = roxmltree::Document::parse(document)
        .map_err(|error| SoapError::Transport(format!("invalid SOAP response: {error}")))?;
    if let Some(fault) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Fault")
    {
        let text = |name: &str| {
            fault
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == name)
                .and_then(|node| node.text())
                .map(|text| text.trim().to_string())
        };
        return Err(SoapError::Fault {
            code: text("errorCode")
                .and_then(|code| code.parse().ok())
                .unwrap_or(0),
            description: text("errorDescription").unwrap_or_default(),
        });
    }
    let expected = format!("{action}Response");
    let response = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == expected)
        .ok_or_else(|| SoapError::Transport(format!("SOAP response has no {expected}")))?;
    Ok(response
        .children()
        .filter(|child| child.is_element())
        .map(|child| {
            (
                child.tag_name().name().to_string(),
                child.text().unwrap_or_default().to_string(),
            )
        })
        .collect())
}

/// Parses the arguments of an incoming SOAP request (used by the test device).
pub fn parse_request(document: &str) -> Result<(String, HashMap<String, String>), String> {
    let document = roxmltree::Document::parse(document).map_err(|error| error.to_string())?;
    let body = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Body")
        .ok_or_else(|| "SOAP request has no Body".to_string())?;
    let action = body
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "SOAP request has no action".to_string())?;
    Ok((
        action.tag_name().name().to_string(),
        action
            .children()
            .filter(|child| child.is_element())
            .map(|child| {
                (
                    child.tag_name().name().to_string(),
                    child.text().unwrap_or_default().to_string(),
                )
            })
            .collect(),
    ))
}

/// HTTP client for SOAP calls and UPnP eventing.
#[derive(Clone)]
pub struct SoapClient {
    http: reqwest::Client,
}

impl SoapClient {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(SOAP_TIMEOUT)
            .connect_timeout(Duration::from_secs(2))
            .user_agent("Parson/1.0 UPnP/1.1")
            .no_proxy()
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { http })
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Calls `action` on `service` at `base_url` (e.g. `http://192.168.1.20:1400`).
    pub async fn call(
        &self,
        base_url: &str,
        service: Service,
        action: &str,
        arguments: &[(&str, &str)],
    ) -> Result<HashMap<String, String>, SoapError> {
        let response = self
            .http
            .post(format!("{base_url}{}", service.control_path()))
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("SOAPACTION", soap_action_header(service, action))
            .body(envelope(service, action, arguments))
            .send()
            .await
            .map_err(|error| SoapError::Transport(error.to_string()))?;
        let body = read_bounded(response).await?;
        parse_response(&body, action)
    }

    /// Fetches a small XML document such as the device description.
    pub async fn get_text(&self, url: &str) -> Result<String, SoapError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|error| SoapError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(SoapError::Transport(format!(
                "GET {url} returned {}",
                response.status()
            )));
        }
        read_bounded(response).await
    }

    /// Subscribes to UPnP events (GENA); returns the subscription id.
    pub async fn subscribe(
        &self,
        base_url: &str,
        service: Service,
        callback: &str,
        timeout_seconds: u32,
    ) -> Result<String, SoapError> {
        let method = reqwest::Method::from_bytes(b"SUBSCRIBE")
            .map_err(|error| SoapError::Transport(error.to_string()))?;
        let response = self
            .http
            .request(method, format!("{base_url}{}", service.event_path()))
            .header("CALLBACK", format!("<{callback}>"))
            .header("NT", "upnp:event")
            .header("TIMEOUT", format!("Second-{timeout_seconds}"))
            .send()
            .await
            .map_err(|error| SoapError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(SoapError::Transport(format!(
                "SUBSCRIBE returned {}",
                response.status()
            )));
        }
        response
            .headers()
            .get("SID")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| SoapError::Transport("SUBSCRIBE answer has no SID".to_string()))
    }
}

async fn read_bounded(response: reqwest::Response) -> Result<String, SoapError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SOAP_RESPONSE_BYTES as u64)
    {
        return Err(SoapError::Transport(
            "UPnP response exceeds size limit".to_string(),
        ));
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| SoapError::Transport(error.to_string()))?
    {
        if body.len() + chunk.len() > MAX_SOAP_RESPONSE_BYTES {
            return Err(SoapError::Transport(
                "UPnP response exceeds size limit".to_string(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| SoapError::Transport("UPnP response is not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::{Service, SoapError, envelope, parse_request, parse_response, soap_action_header};

    #[test]
    fn envelopes_escape_arguments_and_round_trip() {
        let body = envelope(
            Service::AVTransport,
            "SetAVTransportURI",
            &[
                ("InstanceID", "0"),
                ("CurrentURI", "http://h/x?a=1&b=2"),
                ("CurrentURIMetaData", "<DIDL-Lite/>"),
            ],
        );
        assert!(body.contains(
            r#"<u:SetAVTransportURI xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">"#
        ));
        assert!(body.contains("<CurrentURI>http://h/x?a=1&amp;b=2</CurrentURI>"));
        assert!(body.contains("<CurrentURIMetaData>&lt;DIDL-Lite/&gt;</CurrentURIMetaData>"));
        let (action, arguments) = parse_request(&body).expect("request");
        assert_eq!(action, "SetAVTransportURI");
        assert_eq!(arguments["CurrentURI"], "http://h/x?a=1&b=2");
        assert_eq!(arguments["CurrentURIMetaData"], "<DIDL-Lite/>");
        assert_eq!(
            soap_action_header(Service::RenderingControl, "SetVolume"),
            "\"urn:schemas-upnp-org:service:RenderingControl:1#SetVolume\""
        );
    }

    #[test]
    fn parses_outputs_and_upnp_faults() {
        let ok = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:GetVolumeResponse xmlns:u="urn:schemas-upnp-org:service:RenderingControl:1"><CurrentVolume>27</CurrentVolume></u:GetVolumeResponse></s:Body></s:Envelope>"#;
        assert_eq!(
            parse_response(ok, "GetVolume").expect("volume")["CurrentVolume"],
            "27"
        );
        let fault = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>701</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#;
        assert_eq!(
            parse_response(fault, "Play"),
            Err(SoapError::Fault {
                code: 701,
                description: String::new()
            })
        );
        assert!(matches!(
            parse_response(ok, "GetMute"),
            Err(SoapError::Transport(_))
        ));
    }
}
