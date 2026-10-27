# Mujin Music server and remotes

Mujin Music has two parts:

- **The server** holds your library and its database, reads your music
  files, streams them (bit-perfect when nothing needs converting), and
  drives every zone: the speakers and DACs on the server, Mujin Bridges,
  Sonos, UPnP, AirPlay and Chromecast.
- **Remotes** are the apps you browse and play from: Mujin Music Desktop, the
  Android app, and any web browser. Every remote shows the same library,
  queue and zones, and they stay in sync.

One computer can be both. Mujin Music Desktop can install the server as a
background service on the same computer, and the desktop window is then a
remote for it like any other. A headless machine (a NAS, a Raspberry Pi, a
home server, a Windows PC in a cupboard) can run only the server.

## Choosing on Mujin Music Desktop

On first start, Mujin Music Desktop asks where your music lives:

- **Run the server on this computer.** Mujin Music Desktop installs the server as
  a background service (see below) and connects to it on `127.0.0.1`. The
  window is only a remote: close or quit it and phones, browsers and other
  computers keep their library and zones. Settings → Connections shows the
  server's network address, a QR code for your phone, and controls to start,
  stop or restart the server and to launch it at startup.
- **Connect to a server on my network.** The desktop app becomes a remote.
  It never starts a server of its own. Servers on your network show up on
  their own (Bonjour/mDNS, `_parson._tcp`). You can also type an address such
  as `192.168.1.20`, `music-nas.local` or `https://music.example.com`.

Mujin Music Desktop remembers every server you connect to. Each one keeps its own
sign-in. To change server later, open Settings → Connections → **Switch
server**. **Forget** removes a server and signs you out of it on that
computer.

If the server goes away (it restarts, the NAS goes to sleep, the Wi-Fi
drops), Mujin Music Desktop shows that it is reconnecting. When the server comes
back, Mujin Music Desktop returns to the page you were on. If a different Mujin Music
library answers at the same address, Mujin Music asks you before connecting to
it.

### The background server on a desktop computer

| System  | How the server runs                                                                                                                                                                                                                                                                   |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | The Mujin Music server tray host (`ParsonMusicServer.exe`, shipped with Mujin Music Desktop) is copied to `%LOCALAPPDATA%\Programs\Parson Server` and starts at sign-in from `HKCU\…\Run` (its **Start with Windows** item).                                                          |
| macOS   | A LaunchAgent, `~/Library/LaunchAgents/com.parsonlabs.parson.server.plist`, runs Mujin Music in `--server-host` mode. A menu-bar item shows that it is running and offers **Launch at startup** and **Quit Mujin Music Server**. launchd restarts it after a crash, never after Quit. |
| Linux   | A systemd user service, `~/.config/systemd/user/parson-server.service` (`systemctl --user status parson-server`). Without a systemd user session, an XDG autostart entry runs Mujin Music in `--server-host` mode instead.                                                            |

All three use the same data folder as before, so an existing library carries
over. Mujin Music Desktop leaves `server.env` (mode 600) in the data folder: it
holds a key that lets the desktop recognise the server it installed, and
`PARSON_LOCAL_AUDIO=on`, so this computer's DACs are zones. Edit that line to
turn them off; Mujin Music keeps your change. A systemd user service stops when
you sign out unless lingering is on: `loginctl enable-linger $USER`.

What has been tested: the Linux background host end to end (pick “Use this
computer”, pair a phone over the API, quit Mujin Music Desktop, and the phone's
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

On the server, open **Settings → Connections** in any Mujin Music app. It shows:

- the server's address on your home network, and on a VPN such as Tailscale
  if there is one;
- a QR code. Choose **Mujin Music app** to open the Android app already
  connected to this server, or **Browser** to open the web app on any phone
  without installing anything;
- a field for the six-digit pairing code. The phone app signs in with this
  code, so you don't type your password on the phone.

The Android app also finds servers on the same Wi-Fi by itself. It keeps a
list of your libraries in Settings → Devices, so moving between, say, a home
NAS and a studio computer takes one tap.

## Away from home

Settings → Connections → **Away from home** (administrators) sets up how the
library is reached outside the home network. The Mujin Music phone app learns these
addresses from the server after it signs in at home, and switches to them by
itself when the home address stops answering. It goes back to the home
address the next time it connects at home. Your sign-in is only ever sent to
addresses the signed-in server handed out.

There are three ways in. The first is the best for most people:

1. **Tailscale or another VPN (recommended).** Install
   [Tailscale](https://tailscale.com/download) on the server and on your phone,
   and sign both in to the same tailnet. Nothing is opened to the internet and
   the connection is encrypted. Mujin Music sees the VPN address (100.x.y.z) and,
   when the `tailscale` command is available, the machine's MagicDNS name, and
   offers both to the phone. WireGuard and other VPNs work the same way.
2. **My address.** Use this when you already run a reverse proxy with HTTPS
   (Caddy, nginx, Traefik, Synology's reverse proxy) or have a dynamic-DNS
   name. Enter the address, for example `https://music.example.com`. With
   Caddy the whole configuration is:

   ```
   music.example.com {
     reverse_proxy 127.0.0.1:1993
   }
   ```

   Also set `PARSON_PUBLIC_URL=https://music.example.com` on the server, so
   that browser sign-ins over that address use secure cookies.

3. **Router.** Mujin Music asks your router to forward a port to the server with
   UPnP (Internet Gateway Device v1/v2, `WANIPConnection`/`WANPPPConnection`).
   It renews the forward every 30 minutes with a one-hour lease, so the
   forward lapses on its own if the server stops. If the router only allows
   permanent forwards, Mujin Music removes the forward when you switch this off.
   This uses plain HTTP, so sign-ins and music cross the internet
   unencrypted. Use it to get going, and use Tailscale or your own HTTPS
   address every day. If the router reports a shared or private public
   address (carrier-grade NAT, `100.64.0.0/10`, or a second router in front),
   Mujin Music says that a forward can't work there and doesn't offer it to phones.

**Check connection** connects to every address and confirms that it reaches
_this_ server. It doesn't just accept anything that answers: a different
Mujin Music server, a router login page or a misconfigured proxy each fail with a
reason. The check runs from the server, so a router port forward is tested
through the router's own public address. Some routers can't loop a
connection back to themselves. If that check fails, open the Mujin Music app on
mobile data: **Settings → Devices → Check from here** runs the same check from
the phone.

The server can only test its addresses from inside your home, so its check
never says an address works _away_ from home. A VPN address that answers is
shown as reachable over the VPN (for devices on the same VPN). Any other
address that answers is shown as **Leads here, not yet confirmed from
outside** until a phone confirms it on mobile data; no latency is shown for a
check the server ran against itself. A private, loopback, link-local,
carrier-grade NAT or documentation address (or a `.local`/`.lan` name, or a
name that resolves only to such addresses) is reported as **Home network
only**, is never offered to phones as an away-from-home address, and **My
address** refuses it when you save it.

What has been tested: the UPnP client was run against a protocol-level fake
router. The fake answers SSDP and serves a real IGD description and the
`AddPortMapping`/`GetExternalIPAddress`/`DeletePortMapping` SOAP actions,
including the "permanent leases only" (725) and "conflict" (718) faults. The
connectivity check was run against live HTTP servers. It has not been run
against a physical consumer router or over a real Tailscale tailnet.

## Moving to a new server and backups

A backup holds the whole library state: accounts, playlists, favourites,
listening history, metadata edits, artwork and the library index. It does not
hold your music files or session secrets.

**Scheduled backups.** Settings → Data → Scheduled backups. Choose daily,
weekly or off, and optionally a folder on another drive or a mounted NAS share
(for example `/mnt/nas/parson-backups` or `\\nas\backups`). Every backup is
written to the server's own `Backups` folder first and then copied to that
folder under a temporary name, so an interrupted copy never looks like a
backup. Mujin Music keeps the newest N copies there. The page shows when the last
copy was made, or why it failed (for example, the share is not mounted).

**Moving the library** (to a NAS, or because the machine is replaced):

1. On the old server: Settings → Data → **Back up now**. In Mujin Music Desktop,
   **Use another server** offers this before you switch.
2. On the new server: when it asks for the first account, choose **Moving
   from another Mujin Music server?** and pick the backup. Mujin Music looks in its own
   backup folder, in the scheduled backup folder, in `/backups` inside Docker,
   and in any folder you type. You can also upload the file.
3. Leave **Take over the old library** on (the default). The new server then
   answers with the old library's identity, so phones and computers that knew
   the old server find this one as the same library at its new address. They
   ask you to sign in once, because a backup never carries sign-in sessions.
   Switch the old server off first. If both stay on for a while, every app
   lists them the same way on every scan: the one it is connected to stays
   "Connected", the new server is the library's home everywhere else
   (servers report when they took a library over), and the other is listed
   separately with a note to switch it off.
   **Also restore away-from-home access** is offered when the backup has one
   (your own address or a router forward). It is off
   by default because the new server may sit on another network; turn it on
   when the new server replaces the old one in the same place.
4. Mujin Music checks the backup, restarts itself to apply it, and keeps the data
   it replaced in `Recovery/before-restore-…` as a rollback copy. It
   re-executes itself in place, so systemd, launchd, Docker, the Windows tray
   and Mujin Music Desktop's background service all keep running it. If the
   server runs in a terminal without a supervisor, it starts again in the
   same terminal.
5. If the music lives somewhere else on the new server, Settings → Library
   marks the old folder **Not found on this server**. Choose **Locate…** and
   pick the new folder (the picker opens at the closest folder above the old
   location that still exists). Mujin Music indexes it, finds each track at the same place
   inside the folder, moves playlists, favourites, history, preferences and
   metadata edits over to it, and only then removes the old folder. It
   reports any tracks that were not in the new folder.

In Docker, mount a folder for backups so that it can be both a scheduled
destination and a place to restore from:

```yaml
volumes:
  - "/path/to/config:/Parson"
  - "/path/to/music:/music:ro"
  - "/path/to/backups:/backups"
```

## Ports and firewalls

| Port | Protocol | Purpose                                                    |
| ---- | -------- | ---------------------------------------------------------- |
| 1993 | TCP      | The Mujin Music app and API (change it with `PARSON_PORT`) |
| 5353 | UDP      | mDNS: remotes find the server automatically                |
| 1900 | UDP      | SSDP: only when Router remote access is on (outgoing)      |

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
    # Host networking lets phones and Mujin Music Desktop find the server via
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
builds it as a CI artifact. Its tray menu can open Mujin Music, start, stop and
restart the server, start it with Windows, and **Connect a phone or another
computer…**, which opens Settings → Connections. Data is stored in
`%LOCALAPPDATA%\Parson`. On first start, allow Mujin Music through Windows
Firewall on private networks, so that remotes can reach it.

## Server settings

| Variable              | Default                                    | Meaning                                                                                                                                              |
| --------------------- | ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PARSON_DATA_DIR`     | platform data folder (`/Parson` in Docker) | Database, caches, logs                                                                                                                               |
| `PARSON_PORT`         | `1993`                                     | HTTP port                                                                                                                                            |
| `PARSON_BIND_ADDRESS` | `0.0.0.0`                                  | `127.0.0.1` makes the server private to this machine                                                                                                 |
| `PARSON_PUBLIC_URL`   |                                            | The address remotes should use behind a reverse proxy, e.g. `https://music.example.com` (enables secure cookies; cast receivers fetch media from it) |
| `PARSON_LIBRARY_NAME` | `Mujin Music Library`                      | Name shown to remotes and in discovery                                                                                                               |
| `PARSON_LOCAL_AUDIO`  | off                                        | `on` makes this machine's audio devices zones                                                                                                        |

`PARSON_LIBRARY_NAME` and `PARSON_LOCAL_AUDIO` can also be set in `server.env`
in the data folder (one `KEY=value` per line). The environment wins.

Remotes can check which server they are talking to. The address
`/.well-known/parson` returns the server's name, a stable `instanceId`, and a
`bootId` that changes every time the server restarts. Signed-in apps can read
`/api/core/v1/host` to get the addresses that other devices can use.
