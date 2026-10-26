//! The HTTP stream a UPnP renderer pulls.
//!
//! UPnP and OpenHome renderers fetch audio from a URL themselves, so the
//! server's output chain (decoder, DSP, dither) writes into this stream
//! and the renderer reads it as an endless WAV or LPCM resource. Writes
//! block when the renderer stops reading, so the TCP connection's back
//! pressure paces playback in real time exactly as a sound card would.
//!
//! Renderers often open the URL twice (a probe, then the real fetch) or
//! reconnect once after buffering. The first [`PREFIX_BYTES`] of the body
//! are kept, so a connection that starts while the stream is still within
//! them gets the whole beginning again instead of a stream with a hole and
//! no header. The newest connection always wins; older ones are closed.

use std::collections::VecDeque;
use std::io::{self, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::http::read_head;

/// Body bytes kept for a renderer that reconnects near the start.
pub const PREFIX_BYTES: usize = 1 << 20;

/// How long a renderer has to request the stream after it is told to.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

struct State {
    /// The container header (WAV), sent first on every connection.
    header: Vec<u8>,
    /// The first [`PREFIX_BYTES`] of the body, as delivered.
    prefix: Vec<u8>,
    /// Body bytes written and not yet delivered.
    queue: VecDeque<u8>,
    /// Body bytes taken from the queue by a connection.
    delivered: u64,
    written: u64,
    /// The writer reached the end of the track.
    finished: bool,
    cancelled: bool,
    /// Why the stream was stopped, when it failed rather than being
    /// stopped by the player.
    error: Option<String>,
    /// The connection that may read; older generations close.
    generation: u64,
    connections: u64,
    /// When the current connection started receiving the body, which is
    /// when the renderer starts its own buffering and playback clock.
    streaming_since: Option<Instant>,
    /// The last connection has read everything after `finished`.
    completed: bool,
    /// Bytes per frame: connections take whole frames, so a reconnect
    /// never starts between channels.
    frame_bytes: usize,
    /// Kernel send buffer for each connection, in bytes.
    send_buffer: usize,
    requests: Vec<StreamRequest>,
}

/// One request the renderer made, kept for diagnostics and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamRequest {
    pub method: String,
    pub path: String,
    pub user_agent: Option<String>,
    pub range: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A one-track stream served on its own port.
pub struct PcmStream {
    shared: Arc<Shared>,
    url: String,
    content_type: String,
    capacity: usize,
    byte_rate: u64,
    listener_address: SocketAddr,
    thread: Mutex<Option<JoinHandle<()>>>,
    created: Instant,
}

impl PcmStream {
    /// Listens on `bind_ip` (the address the renderer reaches this machine
    /// at) on an ephemeral port. `capacity` bounds what is buffered ahead
    /// of the renderer; `byte_rate` is the body's bytes per second.
    pub fn start(
        bind_ip: IpAddr,
        path: &str,
        content_type: &str,
        header: Vec<u8>,
        frame_bytes: usize,
        capacity: usize,
        byte_rate: u64,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::new(bind_ip, 0))?;
        let listener_address = listener.local_addr()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                header,
                prefix: Vec::new(),
                queue: VecDeque::new(),
                delivered: 0,
                written: 0,
                finished: false,
                cancelled: false,
                error: None,
                generation: 0,
                connections: 0,
                streaming_since: None,
                completed: false,
                frame_bytes: frame_bytes.max(1),
                // About a quarter second: the kernel wakes a blocked writer
                // only once a third of its buffer is free, so a large
                // buffer turns a steady pull into multi-second bursts.
                send_buffer: (byte_rate / 4).clamp(64 * 1024, 1 << 20) as usize,
                requests: Vec::new(),
            }),
            changed: Condvar::new(),
        });
        let accept_shared = Arc::clone(&shared);
        let served_path = path.to_string();
        let served_type = content_type.to_string();
        let thread = std::thread::Builder::new()
            .name("parson-network-stream".into())
            .spawn(move || accept_loop(&listener, &accept_shared, &served_path, &served_type))?;
        let host = match bind_ip {
            IpAddr::V6(ip) => format!("[{ip}]"),
            IpAddr::V4(ip) => ip.to_string(),
        };
        Ok(Self {
            url: format!("http://{host}:{}{path}", listener_address.port()),
            content_type: content_type.to_string(),
            shared,
            capacity: capacity.max(4096),
            byte_rate: byte_rate.max(1),
            listener_address,
            thread: Mutex::new(Some(thread)),
            created: Instant::now(),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Appends body bytes, blocking while the renderer is behind.
    pub fn push(&self, data: &[u8]) -> io::Result<()> {
        let mut state = self.shared.lock();
        let mut offset = 0;
        while offset < data.len() {
            if state.cancelled {
                return Err(match &state.error {
                    Some(error) => io::Error::other(error.clone()),
                    None => io::Error::new(io::ErrorKind::BrokenPipe, "the stream was stopped"),
                });
            }
            if state.connections == 0 && self.created.elapsed() > CONNECT_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "the renderer did not request the stream within {} seconds (is a firewall blocking port {} on this server?)",
                        CONNECT_TIMEOUT.as_secs(),
                        self.listener_address.port()
                    ),
                ));
            }
            let room = self.capacity.saturating_sub(state.queue.len());
            if room == 0 {
                state = self
                    .shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(100))
                    .map(|(guard, _)| guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner().0);
                continue;
            }
            let take = room.min(data.len() - offset);
            state.queue.extend(&data[offset..offset + take]);
            state.written += take as u64;
            offset += take;
            self.shared.changed.notify_all();
        }
        Ok(())
    }

    /// Body bytes the renderer has played, estimated from when it started
    /// receiving and the stream's byte rate (UPnP renderers report no
    /// position for an endless stream).
    pub fn played_bytes(&self) -> u64 {
        let state = self.shared.lock();
        let Some(since) = state.streaming_since else {
            return 0;
        };
        let elapsed = since.elapsed().as_secs_f64();
        ((elapsed * self.byte_rate as f64) as u64).min(state.written)
    }

    pub fn written_bytes(&self) -> u64 {
        self.shared.lock().written
    }

    /// Ends the body and waits until the renderer has read it all and,
    /// by the byte-rate clock, played it, up to `limit`.
    pub fn finish(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        {
            let mut state = self.shared.lock();
            state.finished = true;
            self.shared.changed.notify_all();
            while !state.completed && !state.cancelled && Instant::now() < deadline {
                if state.connections == 0 && self.created.elapsed() > CONNECT_TIMEOUT {
                    return;
                }
                state = self
                    .shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .map(|(guard, _)| guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner().0);
            }
        }
        while Instant::now() < deadline && self.played_bytes() < self.written_bytes() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Fails the stream: the next write returns `message`.
    pub fn abort(&self, message: String) {
        let mut state = self.shared.lock();
        state.cancelled = true;
        state.error = Some(message);
        self.shared.changed.notify_all();
    }

    /// Stops serving and closes every connection.
    pub fn cancel(&self) {
        {
            let mut state = self.shared.lock();
            state.cancelled = true;
            self.shared.changed.notify_all();
        }
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(&self.listener_address, Duration::from_millis(200));
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }

    pub fn requests(&self) -> Vec<StreamRequest> {
        self.shared.lock().requests.clone()
    }

    pub fn connection_count(&self) -> u64 {
        self.shared.lock().connections
    }
}

impl Drop for PcmStream {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>, path: &str, content_type: &str) {
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    for connection in listener.incoming() {
        if shared.lock().cancelled {
            break;
        }
        let Ok(connection) = connection else {
            continue;
        };
        let shared = Arc::clone(shared);
        let path = path.to_string();
        let content_type = content_type.to_string();
        if let Ok(worker) = std::thread::Builder::new()
            .name("parson-network-stream-conn".into())
            .spawn(move || {
                let _ = serve(connection, &shared, &path, &content_type);
            })
        {
            workers.push(worker);
        }
        workers.retain(|worker| !worker.is_finished());
    }
    for worker in workers {
        let _ = worker.join();
    }
}

fn respond_status(mut connection: &TcpStream, status: &str) -> io::Result<()> {
    connection.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
    )
}

/// Caps the kernel's send buffer for `connection`.
#[cfg(unix)]
fn limit_send_buffer(connection: &TcpStream, bytes: usize) {
    use std::os::fd::AsRawFd;
    let value = libc::c_int::try_from(bytes).unwrap_or(libc::c_int::MAX);
    // SAFETY: the descriptor is an open socket owned by `connection`, and
    // the option value is a live c_int whose size is passed with it.
    unsafe {
        libc::setsockopt(
            connection.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&raw const value).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

#[cfg(not(unix))]
fn limit_send_buffer(_connection: &TcpStream, _bytes: usize) {}

fn serve(
    connection: TcpStream,
    shared: &Arc<Shared>,
    path: &str,
    content_type: &str,
) -> io::Result<()> {
    limit_send_buffer(&connection, shared.lock().send_buffer);
    connection.set_read_timeout(Some(Duration::from_secs(5)))?;
    connection.set_write_timeout(Some(Duration::from_secs(20)))?;
    let mut reader = BufReader::new(connection.try_clone()?);
    let (request_line, headers) = read_head(&mut reader)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    {
        let mut state = shared.lock();
        if state.cancelled {
            return respond_status(&connection, "503 Service Unavailable");
        }
        if state.requests.len() < 32 {
            state.requests.push(StreamRequest {
                method: method.clone(),
                path: target.clone(),
                user_agent: header("User-Agent"),
                range: header("Range"),
            });
        }
    }
    if target.split('?').next() != Some(path) {
        return respond_status(&connection, "404 Not Found");
    }
    if method != "GET" && method != "HEAD" {
        return respond_status(&connection, "405 Method Not Allowed");
    }
    let mut writer = connection;
    writer.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\nCache-Control: no-cache, no-store\r\nAccept-Ranges: none\r\nServer: Parson/1.0 UPnP/1.0\r\ntransferMode.dlna.org: Streaming\r\ncontentFeatures.dlna.org: DLNA.ORG_OP=00;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000\r\n\r\n"
        )
        .as_bytes(),
    )?;
    if method == "HEAD" {
        return Ok(());
    }
    // Take over the stream; replay the beginning when it is still kept.
    let (generation, mut replay) = {
        let mut state = shared.lock();
        state.generation += 1;
        state.connections += 1;
        let mut replay = state.header.clone();
        if state.delivered <= state.prefix.len() as u64 {
            replay.extend_from_slice(&state.prefix);
        }
        state.streaming_since = Some(Instant::now());
        shared.changed.notify_all();
        (state.generation, replay)
    };
    if !replay.is_empty() {
        writer.write_all(&replay)?;
        replay.clear();
    }
    let mut chunk = Vec::with_capacity(64 * 1024);
    let mut kept;
    loop {
        chunk.clear();
        {
            let mut state = shared.lock();
            loop {
                if state.cancelled || state.generation != generation {
                    return Ok(());
                }
                if state.queue.len() >= state.frame_bytes
                    || (state.finished && !state.queue.is_empty())
                {
                    break;
                }
                if state.finished {
                    state.completed = true;
                    shared.changed.notify_all();
                    return Ok(());
                }
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(100))
                    .map(|(guard, _)| guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner().0);
            }
            let frame = state.frame_bytes;
            let mut take = state.queue.len().min(64 * 1024) / frame * frame;
            if take == 0 {
                take = state.queue.len();
            }
            chunk.extend(state.queue.drain(..take));
            state.delivered += take as u64;
            let room = PREFIX_BYTES.saturating_sub(state.prefix.len());
            kept = room.min(chunk.len());
            state.prefix.extend_from_slice(&chunk[..kept]);
            shared.changed.notify_all();
        }
        if let Err(error) = writer.write_all(&chunk) {
            // The renderer went away; give the bytes back so the next
            // connection (a reconnect) does not skip them.
            let mut state = shared.lock();
            if state.generation == generation {
                for byte in chunk.iter().rev() {
                    state.queue.push_front(*byte);
                }
                state.delivered = state.delivered.saturating_sub(chunk.len() as u64);
                let length = state.prefix.len();
                state.prefix.truncate(length - kept);
                shared.changed.notify_all();
            }
            return Err(error);
        }
    }
}

/// A RIFF/WAVE header for an endless PCM stream: sizes are set to the
/// largest values, as streaming WAV producers do.
pub fn wav_header(sample_rate: u32, bits: u16, channels: u16) -> Vec<u8> {
    let block_align = channels * bits.div_ceil(8);
    let byte_rate = sample_rate * u32::from(block_align);
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&u32::MAX.wrapping_sub(8).to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&bits.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(u32::MAX - 44).to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::Ipv4Addr;

    fn fetch(url: &str, method: &str, take: Option<usize>) -> Vec<u8> {
        let parsed = super::super::http::parse_url(url).unwrap();
        let mut stream = TcpStream::connect(parsed.socket_addr().unwrap()).unwrap();
        stream
            .write_all(format!("{method} {} HTTP/1.1\r\nHost: x\r\n\r\n", parsed.path).as_bytes())
            .unwrap();
        let mut reader = BufReader::new(stream);
        let (status, _) = read_head(&mut reader).unwrap();
        assert!(status.contains("200"), "{status}");
        let mut body = Vec::new();
        match take {
            Some(count) => {
                body.resize(count, 0);
                reader.read_exact(&mut body).unwrap();
            }
            None => {
                reader.read_to_end(&mut body).unwrap();
            }
        }
        body
    }

    #[test]
    fn serves_header_and_body_and_replays_the_start_to_a_reconnect() {
        let header = wav_header(44_100, 16, 2);
        let stream = Arc::new(
            PcmStream::start(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                "/t.wav",
                "audio/wav",
                header.clone(),
                4,
                1 << 16,
                176_400,
            )
            .unwrap(),
        );
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let writer_stream = Arc::clone(&stream);
        let writer_body = body.clone();
        let writer = std::thread::spawn(move || {
            writer_stream.push(&writer_body).unwrap();
            writer_stream.finish(Duration::from_secs(5));
        });
        // A probe reads the header and a little, then hangs up.
        let probe = fetch(stream.url(), "GET", Some(1_000));
        assert_eq!(&probe[..44], &header[..]);
        let url = stream.url().to_string();
        let full = fetch(&url, "GET", None);
        writer.join().unwrap();
        assert_eq!(&full[..44], &header[..]);
        assert_eq!(&full[44..], &body[..], "the reconnect got the whole body");
        assert!(stream.connection_count() >= 2);
        let requests = stream.requests();
        assert!(requests.iter().all(|request| request.path == "/t.wav"));
    }

    #[test]
    fn writes_block_until_read_and_fail_without_a_renderer() {
        let mut stream = PcmStream::start(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "/x",
            "audio/L16",
            Vec::new(),
            4,
            4096,
            1000,
        )
        .unwrap();
        stream.created -= CONNECT_TIMEOUT + Duration::from_secs(1);
        let error = stream.push(&[0; 10_000]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        stream.cancel();
        assert_eq!(
            stream.push(&[0; 1]).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn wav_header_describes_the_format() {
        let header = wav_header(96_000, 24, 2);
        assert_eq!(header.len(), 44);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            96_000
        );
        assert_eq!(
            u32::from_le_bytes(header[28..32].try_into().unwrap()),
            96_000 * 6
        );
        assert_eq!(u16::from_le_bytes(header[32..34].try_into().unwrap()), 6);
        assert_eq!(u16::from_le_bytes(header[34..36].try_into().unwrap()), 24);
    }
}
