# Parson server and remotes

Parson has two parts:

- **The server** holds your library and its database, reads your music
  files, streams them (bit-perfect when nothing needs converting), and
  drives every zone: the speakers and DACs on the server, Parson Bridges,
  Sonos, UPnP, AirPlay and Chromecast.
- **Remotes** are the apps you browse and play from: Parson Desktop, the
  Android app, and any web browser. Every remote shows the same library,
  queue and zones, and they stay in sync.

One computer can be both. Parson Desktop can install the server as a
background service on the same computer, and the desktop window is then a
remote for it like any other. A headless machine (a NAS, a Raspberry Pi, a
home server, a Windows PC in a cupboard) can run only the server.

## Choosing on Parson Desktop

On first start, Parson Desktop asks where your music lives:

- **Run the server on this computer.** Parson Desktop installs the server as
  a background service (see below) and connects to it on `127.0.0.1`. The
  window is only a remote: close or quit it and phones, browsers and other
  computers keep their library and zones. Settings → Connections shows the
  server's network address, a QR code for your phone, and controls to start,
  stop or restart the server and to launch it at startup.
- **Connect to a server on my network.** The desktop app becomes a remote.
  It never starts a server of its own. Servers on your network show up on
  their own (Bonjour/mDNS, `_parson._tcp`). You can also type an address such
  as `192.168.1.20`, `music-nas.local` or `https://music.example.com`.

Parson Desktop remembers every server you connect to. Each one keeps its own
sign-in. To change server later, open Settings → Connections → **Switch
server**. **Forget** removes a server and signs you out of it on that
computer.

If the server goes away (it restarts, the NAS goes to sleep, the Wi-Fi
drops), Parson Desktop shows that it is reconnecting. When the server comes
back, Parson Desktop returns to the page you were on. If a different Parson
library answers at the same address, Parson asks you before connecting to
it.

### The background server on a desktop computer

| System  | How the server runs                                                                                                                                                                                                                                                         |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | The Parson server tray host (`ParsonMusicServer.exe`, shipped with Parson Desktop) is copied to `%LOCALAPPDATA%\Programs\Parson Server` and starts at sign-in from `HKCU\…\Run` (its **Start with Windows** item).                                                          |
| macOS   | A LaunchAgent, `~/Library/LaunchAgents/com.parsonlabs.parson.server.plist`, runs Parson in `--server-host` mode. A menu-bar item shows that it is running and offers **Launch at startup** and **Quit Parson Server**. launchd restarts it after a crash, never after Quit. |
| Linux   | A systemd user service, `~/.config/systemd/user/parson-server.service` (`systemctl --user status parson-server`). Without a systemd user session, an XDG autostart entry runs Parson in `--server-host` mode instead.                                                       |

All three use the same data folder as before, so an existing library carries
over. Parson Desktop leaves `server.env` (mode 600) in the data folder: it
holds a key that lets the desktop recognise the server it installed, and
`PARSON_LOCAL_AUDIO=on`, so this computer's DACs are zones. Edit that line to
turn them off; Parson keeps your change. A systemd user service stops when
you sign out unless lingering is on: `loginctl enable-linger $USER`.

What has been tested: the Linux background host end to end (pick “Use this
computer”, pair a phone over the API, quit Parson Desktop, and the phone's
status, library and zone calls still succeed); systemd, launchd and the
Windows tray through their exact command lines against recorded fakes; and
the Windows package is built on every pull request. The macOS menu bar and
the Windows tray have not been run on real hardware by these tests.

Sign-in belongs to a server, not to an address. Two servers on one computer
(different ports) never share a sign-in, and a server you signed in to at
`127.0.0.1` stays signed in when you reach it at its network address.

Command-line options and environment variables (for kiosks, shared machines
and scripted installs):

| Option               | Environment                       | Effect                             |
| -------------------- | --------------------------------- | ---------------------------------- |
| `--server=<address>` | `PARSON_DESKTOP_SERVER=<address>` | Start as a remote for this server  |
| `--local`            | `PARSON_DESKTOP_MODE=local`       | Run the server on this computer    |
| `--choose-server`    |                                   | Show the server chooser            |
|                      | `PARSON_PORT`, `PARSON_DATA_DIR`  | Passed on to the background server |

## Connecting a phone

On the server, open **Settings → Connections** in any Parson app. It shows:

- the server's address on your home network, and on a VPN such as Tailscale
  if there is one;
- a QR code. Choose **Parson app** to open the Android app already
  connected to this server, or **Browser** to open the web app on any phone
  without installing anything;
- a field for the six-digit pairing code. The phone app signs in with this
  code, so you don't type your password on the phone.

The Android app also finds servers on the same Wi-Fi by itself. It keeps a
list of your libraries in Settings → Devices, so moving between, say, a home
NAS and a studio computer takes one tap.

## Ports and firewalls

| Port | Protocol | Purpose                                               |
| ---- | -------- | ----------------------------------------------------- |
| 1993 | TCP      | The Parson app and API (change it with `PARSON_PORT`) |
| 5353 | UDP      | mDNS: remotes find the server automatically           |

Remotes need to reach TCP 1993 on the server. If discovery is blocked (some
guest networks, VLANs, Docker bridge networking), connect by address.

## Headless server on Linux or a NAS (Docker)

The container image is `ghcr.io/parsonlabs/parson:latest`. It is built for
x86_64 and arm64, including the Raspberry Pi 4/5, most Synology and QNAP
models, and Unraid.

```yaml
services:
  parson:
    image: ghcr.io/parsonlabs/parson:latest
    # Host networking lets phones and Parson Desktop find the server via
    # mDNS. With bridge networking, publish 1993 and connect by address.
    network_mode: host
    environment:
      PARSON_LIBRARY_NAME: Living room
    volumes:
      - /path/to/parson-data:/Parson
      - /path/to/music:/music:ro
    restart: unless-stopped
```

```sh
docker compose up -d
```

Open `http://<nas-address>:1993` in a browser, create the administrator
account, and choose `/music` as the music folder. Then connect your remotes
as described above.

Synology DSM: Container Manager → Project → Create, then paste the compose
file. Use a shared folder such as `/volume1/music` for the music.
Unraid: add a container with the image above, set Network Type to `host`,
and map `/Parson` to `/mnt/user/appdata/parson`.

## Headless server on Linux (systemd)

Build the server, or copy `parson-music-server` out of the container image:

```sh
bun install --frozen-lockfile
bun --filter parson-music-web build      # the web app the server embeds
cargo build --release -p parson-music    # target/release/parson-music-server
```

Then install it as a service. The unit is in
[`crates/backend/packaging/parson-music.service`](../crates/backend/packaging/parson-music.service):

```sh
sudo install -m 0755 target/release/parson-music-server /usr/local/bin/
sudo cp crates/backend/packaging/parson-music.service /etc/systemd/system/
sudo systemctl enable --now parson-music
journalctl -u parson-music -f            # logs
```

The data lives in `/var/lib/parson`. Install `ffmpeg` if you want transcoding
for low-bandwidth playback and for speakers that can't play the original
format. Lossless playback to capable zones never transcodes.

## Server only on Windows

`ParsonMusicServer.exe` runs the server from the notification area, without
the desktop interface. Every release includes it, and each pull request
builds it as a CI artifact. Its tray menu can open Parson, start, stop and
restart the server, start it with Windows, and **Connect a phone or another
computer…**, which opens Settings → Connections. Data is stored in
`%LOCALAPPDATA%\Parson`. On first start, allow Parson through Windows
Firewall on private networks, so that remotes can reach it.

## Server settings

| Variable              | Default                                    | Meaning                                                                                                          |
| --------------------- | ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| `PARSON_DATA_DIR`     | platform data folder (`/Parson` in Docker) | Database, caches, logs                                                                                           |
| `PARSON_PORT`         | `1993`                                     | HTTP port                                                                                                        |
| `PARSON_BIND_ADDRESS` | `0.0.0.0`                                  | `127.0.0.1` makes the server private to this machine                                                             |
| `PARSON_PUBLIC_URL`   |                                            | The address remotes should use behind a reverse proxy, e.g. `https://music.example.com` (enables secure cookies) |
| `PARSON_LIBRARY_NAME` | `Parson Library`                           | Name shown to remotes and in discovery                                                                           |
| `PARSON_LOCAL_AUDIO`  | off                                        | `on` makes this machine's audio devices zones                                                                    |

`PARSON_LIBRARY_NAME` and `PARSON_LOCAL_AUDIO` can also be set in `server.env`
in the data folder (one `KEY=value` per line). The environment wins.

Remotes can check which server they are talking to. The address
`/.well-known/parson` returns the server's name, a stable `instanceId`, and a
`bootId` that changes every time the server restarts. Signed-in apps can read
`/api/core/v1/host` to get the addresses that other devices can use.
