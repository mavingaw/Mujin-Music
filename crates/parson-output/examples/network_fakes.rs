//! Runs Parson's protocol-level fake network endpoints until interrupted:
//! an OpenHome streamer, a UPnP AV (DLNA) renderer that only takes
//! `audio/L16` at 44.1 and 48 kHz, and an AirPlay 1 receiver.
//!
//! ```sh
//! cargo run -p parson-output --example network_fakes --features network-fakes -- [bind-ip]
//! ```
//!
//! Add them in Settings → Audio outputs → Add a network player, with the
//! addresses printed below. They prove the protocol paths, not a device.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use parson_output::network::fakes::{
    FakeAirPlayConfig, FakeAirPlayReceiver, FakeRenderer, FakeRendererConfig, sample_icon,
};

fn main() -> std::io::Result<()> {
    let ip: IpAddr = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let bind = SocketAddr::new(ip, 0);
    let streamer = FakeRenderer::start_on(
        FakeRendererConfig {
            name: "Test Bench Streamer".into(),
            model_name: "Fake OpenHome Streamer".into(),
            icon: Some(sample_icon()),
            ..FakeRendererConfig::default()
        },
        bind,
    )?;
    let dlna = FakeRenderer::start_on(
        FakeRendererConfig {
            name: "Test Bench DLNA Renderer".into(),
            model_name: "Fake UPnP AV Renderer".into(),
            udn: "uuid:5061-7273-6f6e-fake-renderer02".into(),
            openhome: false,
            sink_protocol_info: Some(
                "http-get:*:audio/L16;rate=44100;channels=2:DLNA.ORG_PN=LPCM,http-get:*:audio/L16;rate=48000;channels=2:*,http-get:*:audio/mpeg:*".into(),
            ),
            ..FakeRendererConfig::default()
        },
        bind,
    )?;
    let airplay = FakeAirPlayReceiver::start_on(FakeAirPlayConfig::default(), bind)?;
    println!("OpenHome streamer: {}", streamer.description_url());
    println!("UPnP AV renderer:  {}", dlna.description_url());
    println!("AirPlay receiver:  {}", airplay.address);
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let state = streamer.state();
        let air = airplay.state();
        println!(
            "streamer: {} {:?} {} bytes | dlna: {} bytes | airplay: {} packets, {} syncs",
            state.transport_state,
            state.stream_format,
            state.received.len(),
            dlna.state().received.len(),
            air.packets,
            air.sync_packets
        );
    }
}
