//! A small HTTP/1.1 client for the LAN protocols network outputs speak.
//!
//! UPnP descriptions, SOAP control calls and device icons are plain HTTP on
//! the local network, so this needs no TLS and no async runtime: each call
//! is a blocking request with a timeout and a bounded body, made from the
//! playback or probing thread.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// An `http://` URL split into what a request needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl Url {
    /// `host:port`, with IPv6 hosts bracketed.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn socket_addr(&self) -> io::Result<SocketAddr> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the host has no address"))
    }
}

/// Parses an `http://host[:port][/path]` URL.
pub fn parse_url(url: &str) -> Result<Url, String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("HTTP://"))
        .ok_or_else(|| format!("{url} is not an http:// URL"))?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    // Drop any user info.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped
            .find(']')
            .ok_or_else(|| format!("{url} has an unterminated IPv6 address"))?;
        let host = &stripped[..end];
        let port = stripped[end + 1..].strip_prefix(':');
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err(format!("{url} has no host"));
    }
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .map_err(|_| format!("{url} has an invalid port"))?,
        None => 80,
    };
    Ok(Url {
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

/// Resolves `relative` (a control URL or icon path from a description)
/// against `base` (the description's location or `URLBase`).
pub fn resolve(base: &str, relative: &str) -> String {
    let relative = relative.trim();
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    let Ok(base_url) = parse_url(base) else {
        return relative.to_string();
    };
    let origin = format!("http://{}", base_url.authority());
    if relative.starts_with('/') {
        return format!("{origin}{relative}");
    }
    let directory = match base_url.path.split('?').next().unwrap_or("/").rfind('/') {
        Some(index) => &base_url.path[..=index],
        None => "/",
    };
    format!("{origin}{directory}{relative}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Makes one request and reads the whole response, failing when the body
/// is larger than `max_body`.
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
    max_body: usize,
) -> io::Result<Response> {
    let parsed =
        parse_url(url).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let address = parsed.socket_addr()?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut head = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: Parson/1.0 UPnP/1.0\r\n",
        parsed.path,
        parsed.authority()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() || method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    read_response(BufReader::new(stream), method == "HEAD", max_body)
}

/// `GET` with the default limits for descriptions and icons.
pub fn get(url: &str, timeout: Duration, max_body: usize) -> io::Result<Response> {
    request("GET", url, &[], &[], timeout, max_body)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Reads a status line and headers, ending at the blank line.
pub fn read_head<R: BufRead>(reader: &mut R) -> io::Result<(String, Vec<(String, String)>)> {
    let mut first = String::new();
    if reader.read_line(&mut first)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the connection closed before a response",
        ));
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if headers.len() > 100 {
            return Err(invalid("too many headers"));
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    Ok((first.trim_end().to_string(), headers))
}

fn read_response<R: BufRead>(
    mut reader: R,
    head_only: bool,
    max_body: usize,
) -> io::Result<Response> {
    let (status_line, headers) = read_head(&mut reader)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| invalid("malformed status line"))?;
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let mut body = Vec::new();
    if !head_only {
        if header("Transfer-Encoding")
            .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
        {
            loop {
                let mut size_line = String::new();
                reader.read_line(&mut size_line)?;
                let size =
                    usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
                        .map_err(|_| invalid("malformed chunk size"))?;
                if size == 0 {
                    break;
                }
                if body.len() + size > max_body {
                    return Err(invalid("the response is too large"));
                }
                let start = body.len();
                body.resize(start + size, 0);
                reader.read_exact(&mut body[start..])?;
                let mut crlf = [0u8; 2];
                reader.read_exact(&mut crlf)?;
            }
        } else if let Some(length) = header("Content-Length") {
            let length: usize = length
                .parse()
                .map_err(|_| invalid("malformed Content-Length"))?;
            if length > max_body {
                return Err(invalid("the response is too large"));
            }
            body.resize(length, 0);
            reader.read_exact(&mut body)?;
        } else {
            reader.take(max_body as u64 + 1).read_to_end(&mut body)?;
            if body.len() > max_body {
                return Err(invalid("the response is too large"));
            }
        }
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_and_resolves_urls() {
        let url = parse_url("http://192.168.1.40:49152/description.xml").unwrap();
        assert_eq!(url.host, "192.168.1.40");
        assert_eq!(url.port, 49152);
        assert_eq!(url.path, "/description.xml");
        assert_eq!(parse_url("http://renderer").unwrap().port, 80);
        assert_eq!(
            parse_url("http://[fe80::1]:8080/x").unwrap().host,
            "fe80::1"
        );
        assert!(parse_url("https://renderer/").is_err());
        let base = "http://10.0.0.5:1400/xml/device_description.xml";
        assert_eq!(
            resolve(base, "/MediaRenderer/AVTransport/Control"),
            "http://10.0.0.5:1400/MediaRenderer/AVTransport/Control"
        );
        assert_eq!(
            resolve(base, "icon.png"),
            "http://10.0.0.5:1400/xml/icon.png"
        );
        assert_eq!(resolve(base, "http://other/x"), "http://other/x");
    }

    #[test]
    fn reads_length_chunked_and_closed_bodies() {
        let fixed = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let response = read_response(Cursor::new(fixed), false, 100).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        let chunked =
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(
            read_response(Cursor::new(chunked), false, 100)
                .unwrap()
                .body,
            b"abcde"
        );
        let closed = "HTTP/1.0 500 Internal\r\nServer: x\r\n\r\nbody";
        let response = read_response(Cursor::new(closed), false, 100).unwrap();
        assert_eq!(response.status, 500);
        assert_eq!(response.header("server"), Some("x"));
        assert!(read_response(Cursor::new(fixed), false, 3).is_err());
    }
}
