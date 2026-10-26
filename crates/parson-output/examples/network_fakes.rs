//! Runs Parson's protocol-level fake AirPlay 1 receiver until
//! interrupted.
//!
//! ```sh
//! cargo run -p parson-output --example network_fakes --features network-fakes -- [bind-ip]
//! ```
//!
//! Add it in Settings → Audio outputs → Add a network player, with the
//! address printed below. It proves the protocol path, not a device. A fake
//! UPnP or OpenHome renderer runs with the server's zones:
//! `cargo test -p parson-music serve_fake_upnp_renderer -- --ignored`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use parson_output::network::fakes::{FakeAirPlayConfig, FakeAirPlayReceiver};

fn main() -> std::io::Result<()> {
    let ip: IpAddr = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let airplay =
        FakeAirPlayReceiver::start_on(FakeAirPlayConfig::default(), SocketAddr::new(ip, 0))?;
    println!("AirPlay receiver: {}", airplay.address);
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let state = airplay.state();
        println!(
            "airplay: {} packets, {} syncs, {} resent",
            state.packets, state.sync_packets, state.resent
        );
    }
}
