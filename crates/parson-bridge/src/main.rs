//! `parson-bridge`: run it on the machine your DAC is plugged into. Parson
//! finds it on the local network (or add it by address in Parson's zone
//! picker) and every audio output it lists becomes a zone.

use std::net::TcpListener;
use std::sync::Arc;

use parson_bridge::audio::{self, DeviceBackend};
use parson_bridge::endpoint::{Endpoint, EndpointConfig};
use parson_bridge::protocol::DEFAULT_PORT;

const USAGE: &str = "parson-bridge: makes this machine's DACs and audio outputs Parson zones.

USAGE:
    parson-bridge [--name NAME] [--port PORT] [--bind ADDRESS] [--simulated] [--no-mdns]
    parson-bridge --list

OPTIONS:
    --name NAME      Name shown in Parson (default: this computer's name)
    --port PORT      TCP port to listen on (default: 1994)
    --bind ADDRESS   Address to listen on (default: 0.0.0.0)
    --simulated      Two simulated DACs instead of real devices (demos, tests)
    --no-mdns        Do not advertise on the local network; add the bridge
                     in Parson by address instead
    --list           Print the audio devices and their formats, then exit

Environment: PARSON_BRIDGE_ID overrides the stable bridge id;
RUST_LOG=debug shows protocol detail.";

fn host_name() -> String {
    #[cfg(unix)]
    {
        let mut buffer = [0_u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname writes a
        // NUL-terminated name into it (truncated if needed).
        let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
        if result == 0 {
            let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(0);
            if let Ok(name) = std::str::from_utf8(&buffer[..end])
                && !name.is_empty()
            {
                return name.trim_end_matches(".local").to_string();
            }
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Parson Bridge".to_string())
}

/// A random id, kept in the user's config directory so zone settings in
/// Parson follow the bridge across restarts and address changes.
fn bridge_id() -> String {
    if let Ok(id) = std::env::var("PARSON_BRIDGE_ID")
        && !id.trim().is_empty()
    {
        return id.trim().to_string();
    }
    let directory = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .map(|base| base.join("parson-bridge"));
    let path = directory.as_ref().map(|directory| directory.join("id"));
    if let Some(path) = &path
        && let Ok(id) = std::fs::read_to_string(path)
        && !id.trim().is_empty()
    {
        return id.trim().to_string();
    }
    let seed = std::collections::hash_map::RandomState::new();
    let random = {
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = seed.build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0),
        );
        hasher.finish()
    };
    let id = format!("{random:016x}");
    if let (Some(directory), Some(path)) = (directory, path) {
        let _ = std::fs::create_dir_all(directory);
        let _ = std::fs::write(path, &id);
    }
    id
}

fn describe(backend: &dyn DeviceBackend) {
    let devices = backend.devices();
    if devices.is_empty() {
        println!("No audio outputs found ({}).", backend.name());
    }
    for device in devices {
        let rates: Vec<String> = device
            .sample_rates
            .iter()
            .map(|rate| format!("{}", f64::from(*rate) / 1000.0))
            .collect();
        let formats: Vec<&str> = device.formats.iter().map(|format| format.name()).collect();
        println!(
            "{}  [{}]\n    {}\n    {} kHz · {} · up to {} channels · {}{}",
            device.name,
            device.id,
            device.description,
            rates.join(", "),
            formats.join(" / "),
            device.max_channels,
            match device.hardware_volume {
                Some(volume) => format!(
                    "hardware volume {:.1} to {:.1} dB",
                    volume.min_db, volume.max_db
                ),
                None => "no hardware volume".to_string(),
            },
            if device.busy {
                " · in use by another app"
            } else {
                ""
            }
        );
    }
}

#[cfg(feature = "mdns")]
fn advertise(config: &EndpointConfig, port: u16) -> Option<mdns_sd::ServiceDaemon> {
    let daemon = mdns_sd::ServiceDaemon::new().ok()?;
    let host = format!(
        "parson-bridge-{}.local.",
        &config.bridge_id[..8.min(config.bridge_id.len())]
    );
    let properties = std::collections::HashMap::from([
        ("id".to_string(), config.bridge_id.clone()),
        ("name".to_string(), config.name.clone()),
        (
            "protocol".to_string(),
            parson_bridge::protocol::PROTOCOL_VERSION.to_string(),
        ),
        ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
    ]);
    let service = mdns_sd::ServiceInfo::new(
        parson_bridge::protocol::SERVICE_TYPE,
        &config.name,
        &host,
        (),
        port,
        properties,
    )
    .ok()?
    .enable_addr_auto();
    daemon.register(service).ok()?;
    Some(daemon)
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let mut args = std::env::args().skip(1);
    let mut name = None;
    let mut port = DEFAULT_PORT;
    let mut bind = "0.0.0.0".to_string();
    let mut simulated = false;
    let mut mdns = true;
    let mut list = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = args.next(),
            "--port" => match args.next().and_then(|value| value.parse().ok()) {
                Some(value) => port = value,
                None => {
                    eprintln!("--port needs a number\n\n{USAGE}");
                    std::process::exit(2);
                }
            },
            "--bind" => bind = args.next().unwrap_or(bind),
            "--simulated" => simulated = true,
            "--no-mdns" => mdns = false,
            "--list" => list = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other => {
                eprintln!("unknown option {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let (backend, note): (Option<Arc<dyn DeviceBackend>>, Option<String>) = if simulated {
        (
            Some(Arc::new(
                audio::simulated::SimulatedBackend::with_default_devices(1.0),
            )),
            None,
        )
    } else {
        match audio::native_backend() {
            Ok(backend) => (Some(Arc::from(backend)), None),
            Err(error) => (None, Some(error)),
        }
    };
    if list {
        match &backend {
            Some(backend) => describe(backend.as_ref()),
            None => println!("{}.", note.unwrap_or_default()),
        }
        return;
    }
    let host = host_name();
    let config = EndpointConfig {
        bridge_id: bridge_id(),
        name: name.unwrap_or_else(|| host.clone()),
        host,
        clock_offset_us: 0,
        note: note.clone(),
    };
    if let Some(note) = &note {
        tracing::warn!("{note}.");
    }
    let listener = match TcpListener::bind((bind.as_str(), port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("could not listen on {bind}:{port}: {error}");
            std::process::exit(1);
        }
    };
    let endpoint = match Endpoint::start(listener, backend.clone(), config.clone()) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("could not start: {error}");
            std::process::exit(1);
        }
    };
    #[cfg(feature = "mdns")]
    let _advertisement = mdns.then(|| advertise(&config, port)).flatten();
    #[cfg(not(feature = "mdns"))]
    let _ = mdns;
    tracing::info!(
        name = config.name,
        address = %endpoint.address(),
        devices = backend.as_ref().map(|backend| backend.devices().len()).unwrap_or(0),
        "Parson Bridge is ready; Parson finds it on the local network"
    );
    loop {
        std::thread::park();
    }
}
