//! The HTTP-style text the LAN protocols speak: `http://` addresses of
//! players added by address, and the head of an RTSP (AirPlay) response,
//! which has HTTP's shape. Blocking, with no TLS and no async runtime.

use std::io::{self, BufRead};

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_urls() {
        let url = parse_url("http://192.168.1.40:49152/description.xml").unwrap();
        assert_eq!(url.host, "192.168.1.40");
        assert_eq!(url.port, 49152);
        assert_eq!(url.path, "/description.xml");
        assert_eq!(url.authority(), "192.168.1.40:49152");
        assert_eq!(parse_url("http://renderer").unwrap().port, 80);
        assert_eq!(
            parse_url("http://[fe80::1]:8080/x").unwrap().authority(),
            "[fe80::1]:8080"
        );
        assert!(parse_url("https://renderer/").is_err());
    }

    #[test]
    fn reads_a_response_head() {
        let mut reader =
            Cursor::new("RTSP/1.0 200 OK\r\nCSeq: 1\r\nServer: AirTunes/366.0\r\n\r\nbody");
        let (status, headers) = read_head(&mut reader).unwrap();
        assert_eq!(status, "RTSP/1.0 200 OK");
        assert_eq!(
            headers[1],
            ("Server".to_string(), "AirTunes/366.0".to_string())
        );
        assert!(read_head(&mut Cursor::new("")).is_err());
    }
}
